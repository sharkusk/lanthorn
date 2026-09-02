//! A reproducible gallery of presentable frames (SQ-0942).
//!
//! WHAT THIS IS FOR. A project page needs pictures, and a terminal app is hard
//! to sell in stills. [`super::driver`] already boots the REAL binary under a
//! pty from a command line, and [`super::raster`] already draws the screen a
//! terminal would resolve out of the bytes it wrote back. What was missing was
//! a tool whose output is meant to be LOOKED at rather than measured, and whose
//! whole set regenerates from one file each release so the page cannot drift
//! from the build.
//!
//! WHY IT IS NOT THE ORACLE WITH A NICER FONT. `raster` is a geometry oracle and
//! its own docs are emphatic that it is not a screenshot. Handing THAT a real
//! typeface would be actively harmful: it would stop looking synthetic while
//! still not being anyone's terminal — no hinting, wrong face, wrong metrics —
//! and a picture that is 90% convincing invites exactly the judgement it cannot
//! support. So the font lives here, behind a flag the tests never pass, and
//! every frame this module writes carries a burnt-in label saying what it is.
//! [`label`] is not decoration; it is the reason a real face is allowed at all.
//!
//! THE RECIPE IS THE COMMITTED ARTEFACT. `examples/gallery.toml` is the input;
//! the PNGs are output and belong under `target/`. Nothing here records a
//! release number or a turn count by hand — the release and serial are read out
//! of the header of the bytes the medium actually mounted, and the turn count is
//! counted off the key spec, so neither can drift from the frame it describes.
//!
//! THE TWO TRAPS, ENCODED. A capture that does not negotiate kitty silently
//! measures the half-block backend, so [`Backend::Kitty`] shots FAIL rather than
//! quietly produce a picture of the wrong renderer. And the half-block picker
//! uses its own 10x20 font whatever the terminal reports, so [`Shot::cell`]
//! chooses the cell size from the backend rather than letting a manifest author
//! pick one that renders a geometry lanthorn was not using.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use image::{Rgba, RgbaImage};
use serde::Deserialize;

use super::driver::{self, Capture, Key, Spec};

/// The seed pinned into every shot's config unless the manifest overrides it.
///
/// Not cosmetic. fmvpoker and scopa deal randomly, and an unpinned gallery
/// regenerates differently every release for no reason at all: a first
/// comparison of two fmvpoker frames showed 37,097 differing pixels that were
/// entirely a different card deal, and none of them a render change.
pub const DEFAULT_SEED: u32 = 12345;

/// Which graphics backend a shot is taken through.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Backend {
    /// The default and the one the app ships for: art as kitty placements, text
    /// as terminal cells. A shot that fails to negotiate it is discarded.
    #[default]
    Kitty,
    /// The universal fallback: the same PIXEL path resolved into `▀`/`▄` cells.
    /// Worth a gallery row because it is what a reader on a terminal without
    /// graphics actually gets, and because it is the only v6 output an
    /// asciinema cast can carry (SQ-0943).
    Halfblocks,
}

impl Backend {
    pub fn as_str(self) -> &'static str {
        match self {
            Backend::Kitty => "kitty",
            Backend::Halfblocks => "halfblocks",
        }
    }
}

/// One frame in the gallery, exactly as `examples/gallery.toml` spells it.
///
/// Deliberately absent: the release, the serial and the turn count. All three
/// are DERIVED — the first two from the mounted story's header, the third from
/// [`Shot::keys`] — because a hand-written provenance line is a second copy of
/// the truth and this repo has been bitten by one before.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Shot {
    /// Stable slug; becomes the PNG's filename, so it must survive being one.
    pub id: String,
    /// The game, for the caption.
    pub title: String,
    /// Which pressing — "Amiga floppy (.adf)", "Blorb", "Apple II 5.25-inch".
    /// A prose label for the reader; the machine-checkable half is the release
    /// read off the header at capture time.
    pub press: String,
    /// One or two sentences under the frame. The page is a gallery with
    /// captions, so this may be longer than the surrounding body text.
    pub caption: String,
    /// Path to the medium, relative to the repository root. A DIRECTORY when
    /// [`Shot::library`] is set, a file otherwise.
    pub media: String,
    /// `media` is a story LIBRARY — a directory — and the frame is the picker
    /// rather than a game (SQ-1080).
    ///
    /// Declared rather than sniffed off the disk, so a manifest validates the
    /// same on a machine that has the directory and on one that does not:
    /// whether a shot is a library launch is a fact about the shot, not about
    /// this checkout.
    ///
    /// Everything a story shot derives from its bytes is meaningless here
    /// because NO STORY HAS BEEN CHOSEN — there is no release, no serial, no
    /// medium, no native screen and no rendition to name. [`Shot::validate`]
    /// refuses each of those fields on a library shot rather than let one be
    /// silently ignored, and [`Provenance`] carries a separate arm that says
    /// what a directory launch actually knows about itself.
    #[serde(default)]
    pub library: bool,
    /// The key spec, in [`Key::parse`]'s spelling: `cr,wait:900,text:look,cr`.
    pub keys: String,
    /// Terminal size in cells, `COLSxROWS`.
    pub size: String,
    /// Which backend to capture through.
    #[serde(default)]
    pub backend: Backend,
    /// Extra arguments passed through to lanthorn. The tool owns `--user-dir`
    /// and `--image-protocol`; naming either here is a manifest error.
    #[serde(default)]
    pub args: Vec<String>,
    /// The PRNG seed pinned for this shot.
    #[serde(default = "default_seed")]
    pub seed: u32,
    /// Keep the map pane. Off by default, so the story pane owns the frame.
    #[serde(default)]
    pub show_map: bool,
    /// Text that MUST be on the resolved screen, or the shot is discarded.
    ///
    /// The non-vacuity guard, and it earns its place. Pointed at a disk image
    /// holding several games, lanthorn opens a browser, and two blank keypresses
    /// picked *Ballyhoo* off a neighbouring floppy while every number in the
    /// record — release, serial, medium — went on describing the Zork Zero image
    /// the manifest named, because those are read from the file and not from the
    /// frame. Arthur's ProDOS press is the same failure more quietly: it renders
    /// identically at 6 and 40 keypresses because it never answers the restore
    /// question, and the still is of a boot prompt. One string off the screen
    /// catches both.
    #[serde(default)]
    pub expect: Vec<String>,
    /// Pin the v6 render mode for this shot, or `None` to take the shipped
    /// default (SQ-1009, SQ-1152).
    ///
    /// Hybrid draws text as terminal cells, so a shot meant to show a face the
    /// RELEASE shipped — Arthur's proportional Amiga typeface — cannot show it in
    /// hybrid at all: the glyphs on screen would be the terminal's. Written into
    /// the shot's own config beside the seed, which is the only channel the
    /// manifest has into a run.
    ///
    /// This was `raster = true` until `extended` needed a row of its own. A second
    /// bool could have expressed "raster and extended at once", which is not a
    /// state — the modes are one choice, so they are one field, spelled with the
    /// same tokens `~/.lanthorn/config.toml` uses because it is the same key.
    /// `None` rather than `Some(Hybrid)` is what keeps the library refusal below
    /// able to tell an author who asked for a mode from one who said nothing.
    #[serde(default)]
    pub v6_render: Option<app::config::V6RenderMode>,
    /// The least number of cells a placement must actually cover.
    ///
    /// The guard for a frame with no text in it. Scopa and FMV Poker draw the
    /// whole screen as one composite — their buttons and their prompts are
    /// PICTURES — so a substring search over the cells finds nothing at all and
    /// would have to be waived. What those frames can assert instead is that the
    /// art landed, which is the same question SQ-0934 spent three rounds on when
    /// a cell harness reported "no art inside the viewport" and was believed.
    #[serde(default)]
    pub expect_art_cells: usize,
    /// The least number of cells of the STORY PANE that must carry a letter or a
    /// digit the game wrote (SQ-1164).
    ///
    /// The mirror of [`Shot::expect_art_cells`], and it exists because `expect`
    /// cannot ask this question. All five Journey shots drove `n` and three blank
    /// returns, which halts at Journey's opening MENU — the illustration on the
    /// left, an empty text pane on the right, and *Start · Background · Change
    /// Name · Help · Game* along the bottom. Every string those shots named ("The
    /// Party", "Individual Commands") is a heading on that menu, so the guard
    /// passed on a frame whose whole subject — the prose — was missing, and it
    /// took a human eye on a proof sheet to catch it.
    ///
    /// Measured on `journey-blorb` at 82x28: the menu frame scores **59**, the
    /// title splash 170, the intro card 350, and the frame in play **517**.
    ///
    /// INSIDE THE PANE, which is the whole of why this is not a screen-wide letter
    /// count: lanthorn's own chrome — the header naming the story and the medium,
    /// the `Ctrl+P: menu` help bar — is a hundred-odd letters that are there
    /// whatever the game did, so counting the screen is vacuous by construction.
    /// Cells under a kitty placement are art and are skipped; a half-block frame's
    /// art is `▀`/`▄`, which is not alphanumeric and never counted either.
    #[serde(default)]
    pub expect_prose_cells: usize,
    /// Capture this recipe once per §11.1.3 interpreter number and TILE the
    /// results into a single frame (SQ-1165).
    ///
    /// **A composite is a shot KIND, not a second tool.** Every other row here
    /// renders one frame; this one renders N and lays them out. It could have
    /// been an example of its own, and that would have been worse: the
    /// provenance read off the mount, `expect`, [`Shot::expect_prose_cells`],
    /// the burnt-in label, the proof sheet and `gallery.json` are all things a
    /// composite needs exactly as much as a single frame does, and a second
    /// renderer would have reimplemented them and then drifted. So the only new
    /// thing is this field and the tiling under it.
    ///
    /// Each tile launches with `--interpreter N --colour machine` appended,
    /// which is the pair SQ-1154 made reach a bare story file: `machine` is the
    /// opt-in that says you mean the number you typed, and without it a plain
    /// `.z3` falls through to the theme and every tile comes out the same.
    /// Everything else about the run — the story, the size, the keys, the seed —
    /// is shared, because the frame's whole claim is that the MACHINE is the only
    /// variable.
    ///
    /// The non-vacuity guard for this kind is [`check_machines_differ`], and it
    /// is the reason the row is worth having: `expect` names a string, and six
    /// copies of one palette all say it.
    #[serde(default)]
    pub machines: Vec<u8>,
}

fn default_seed() -> u32 {
    DEFAULT_SEED
}

/// The whole manifest.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub shots: Vec<Shot>,
    /// The story libraries a `library = true` shot can open (SQ-1080).
    #[serde(default)]
    pub libraries: Vec<Library>,
}

impl Manifest {
    /// Parse and validate. Every error a manifest author can plausibly make is
    /// caught here rather than three minutes into a capture run — and a test
    /// runs this over the committed file, so a broken manifest fails the gate
    /// instead of failing whoever next regenerates the page.
    pub fn parse(text: &str) -> Result<Manifest, String> {
        let m: Manifest = toml::from_str(text).map_err(|e| format!("gallery manifest: {e}"))?;
        if m.shots.is_empty() {
            return Err("gallery manifest: no [[shots]] — an empty gallery is a mistake, not a choice".into());
        }
        let mut libs: BTreeSet<&str> = BTreeSet::new();
        for l in &m.libraries {
            l.validate()?;
            if !libs.insert(l.id.as_str()) {
                return Err(format!("gallery manifest: duplicate library id `{}`", l.id));
            }
        }
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        for s in &m.shots {
            s.validate()?;
            if !seen.insert(s.id.as_str()) {
                return Err(format!("gallery manifest: duplicate shot id `{}` — ids are filenames", s.id));
            }
            // A library shot names a library the way every other shot names a
            // file, and an unresolvable name is caught HERE rather than as a
            // missing directory three minutes into a run.
            m.subject(s)?;
        }
        Ok(m)
    }

    /// What one shot opens: a medium on disk, or one of the libraries above.
    ///
    /// Resolved once, as a value, because the two answers are not
    /// interchangeable and every consumer needs the same one — the provenance
    /// under the frame, the path the binary is launched with, and the config the
    /// run is given all follow from it. A function that took a path and a bool
    /// instead would let a caller supply half of it and get a plausible frame
    /// (see CLAUDE.md's refactoring policy, which this file has been bitten by
    /// twice).
    pub fn subject<'a>(&'a self, shot: &Shot) -> Result<Subject<'a>, String> {
        if !shot.library {
            return Ok(Subject::Medium(shot.media_path()));
        }
        self.libraries
            .iter()
            .find(|l| l.id == shot.media)
            .map(Subject::Library)
            .ok_or_else(|| {
                format!(
                    "gallery manifest: `{}` is a library shot whose `media = {:?}` names no \
                     [[libraries]] entry (declared: {})",
                    shot.id,
                    shot.media,
                    if self.libraries.is_empty() {
                        "none".to_string()
                    } else {
                        self.libraries.iter().map(|l| l.id.as_str()).collect::<Vec<_>>().join(", ")
                    }
                )
            })
    }

    /// The committed manifest's path: `crates/app/examples/gallery.toml`.
    pub fn default_path() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/gallery.toml")
    }

    /// Read and parse the committed manifest.
    pub fn load(path: &Path) -> Result<Manifest, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("gallery manifest: reading {}: {e}", path.display()))?;
        Manifest::parse(&text)
    }
}

/// A story library a shot can point the picker at (SQ-1080).
///
/// **Named members, not a directory off this machine.** Pointing a shot straight
/// at `stories/` was the first thing tried and it is the wrong fixture twice
/// over: the frame would show whatever that folder happens to hold on the day —
/// 287 entries here, a different number and a different first screenful on the
/// next machine — and the picker sorts by title, so the screenful the shot lands
/// on is decided by the alphabet rather than by the manifest. A frame is a
/// fixture; this is how a library one gets named.
///
/// The harness stages the members as symlinks into a throwaway directory per
/// capture, so nothing is copied and nothing under `from` is touched.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Library {
    /// How a shot names this library: its `media`.
    pub id: String,
    /// The directory the members are drawn from, relative to the repo root.
    pub from: String,
    /// Filenames within `from`. A member that is not there is a reported
    /// failure, exactly as a missing medium is.
    pub members: Vec<String>,
}

impl Library {
    fn validate(&self) -> Result<(), String> {
        if self.id.is_empty()
            || !self.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Err(format!(
                "gallery manifest: library id `{}` must be lowercase ASCII, digits and dashes",
                self.id
            ));
        }
        if self.from.trim().is_empty() {
            return Err(format!("gallery manifest: library `{}` has an empty `from`", self.id));
        }
        if self.members.is_empty() {
            return Err(format!(
                "gallery manifest: library `{}` has no members — an empty picker is not a picture of a catalogue",
                self.id
            ));
        }
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        for m in &self.members {
            if m.trim().is_empty() || m.contains('/') {
                return Err(format!(
                    "gallery manifest: library `{}` member {m:?} must be a bare filename inside `{}`",
                    self.id, self.from
                ));
            }
            if !seen.insert(m.as_str()) {
                return Err(format!("gallery manifest: library `{}` names {m:?} twice", self.id));
            }
        }
        Ok(())
    }

    /// Build the directory the picker opens: one symlink per member, under
    /// `work`.
    ///
    /// Rebuilt from scratch every capture. A member left behind by an earlier
    /// run of an earlier manifest is a row in the catalogue that the committed
    /// file does not name, and a frame nobody can reproduce.
    pub fn stage(&self, work: &Path) -> Result<PathBuf, String> {
        let from = repo_root().join(&self.from);
        // `<work>/lib/<id>`, not `<work>/library-<id>`, because the directory's
        // own NAME ends up on screen: the picker's header says what it scanned.
        // The nesting is what keeps that name free of a disambiguating prefix
        // while still not colliding with the per-shot user dirs beside it.
        let dir = work.join("lib").join(&self.id);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).map_err(|e| format!("library `{}`: {}: {e}", self.id, dir.display()))?;
        for m in &self.members {
            let src = from.join(m);
            if !src.is_file() {
                return Err(format!(
                    "library `{}`: no member at {} (the media directories are gitignored)",
                    self.id,
                    src.display()
                ));
            }
            std::os::unix::fs::symlink(&src, dir.join(m))
                .map_err(|e| format!("library `{}`: linking {m:?}: {e}", self.id))?;
        }
        Ok(dir)
    }
}

/// What a shot opens — see [`Manifest::subject`].
#[derive(Clone, Debug)]
pub enum Subject<'a> {
    /// One file on disk, mounted the way the app mounts it.
    Medium(PathBuf),
    /// A library of stories, staged at capture time. No story is chosen.
    Library(&'a Library),
}

/// The repository root, from this crate's manifest directory.
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

impl Shot {
    fn validate(&self) -> Result<(), String> {
        let who = &self.id;
        if self.id.is_empty()
            || !self.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Err(format!(
                "gallery manifest: shot id `{who}` must be lowercase ASCII, digits and dashes — it becomes a filename"
            ));
        }
        for (field, value) in [("title", &self.title), ("press", &self.press), ("caption", &self.caption), ("media", &self.media)] {
            if value.trim().is_empty() {
                return Err(format!("gallery manifest: `{who}` has an empty `{field}`"));
            }
        }
        self.size_cells().map(|_| ())?;
        self.keys().map(|_| ())?;
        // A half-block frame's art IS cells: `▀` with two colours, and not a
        // placement anywhere in the stream. `expect_art_cells` counts placements,
        // so on this backend it can only ever read zero and would fail every
        // shot that set it. The guard a half-block shot wants is `▀` in `expect`.
        if self.backend == Backend::Halfblocks && self.expect_art_cells > 0 {
            return Err(format!(
                "gallery manifest: `{who}` is a half-block shot with `expect_art_cells` — half-blocks \
                 emit no placements at all, so that count is always zero. Put `\u{2580}` in `expect` instead"
            ));
        }
        // A LIBRARY SHOT HAS NO STORY, so every field that describes one is a
        // field that would be quietly ignored (SQ-1080). `v6_render` names a v6
        // render mode and the picker is not a v6 screen; `--pictures` names a
        // rendition of artwork inside a story nobody has opened; `show_map` writes
        // a per-game sidecar keyed on the story path, and here that path is a
        // directory. Refusing them is the difference between a manifest that
        // means what it says and one whose author believes a line that does
        // nothing.
        if self.library {
            for (field, set) in [("v6_render", self.v6_render.is_some()), ("show_map", self.show_map)] {
                if set {
                    return Err(format!(
                        "gallery manifest: `{who}` is a library shot with `{field}` — no story has \
                         been chosen yet, so there is nothing for it to apply to"
                    ));
                }
            }
            if self.pictures().is_some() {
                return Err(format!(
                    "gallery manifest: `{who}` is a library shot passing `--pictures` — that names a \
                     rendition of artwork inside a story the picker has not opened"
                ));
            }
            if self.expect_prose_cells > 0 {
                return Err(format!(
                    "gallery manifest: `{who}` is a library shot with `expect_prose_cells` — that \
                     counts the letters a STORY wrote into the pane, and the picker has not opened one"
                ));
            }
            if !self.machines.is_empty() {
                return Err(format!(
                    "gallery manifest: `{who}` is a library shot with `machines` — an interpreter \
                     number is what a STORY's header is told, and the picker has not opened one"
                ));
            }
        }
        // ── A composite shot (SQ-1165) ───────────────────────────────────────
        if !self.machines.is_empty() {
            // ONE TILE IS NOT A COMPARISON. The whole subject of this kind of
            // frame is that the machines differ, which a single tile cannot
            // show and `check_machines_differ` cannot test.
            if self.machines.len() < 2 {
                return Err(format!(
                    "gallery manifest: `{who}` names {} machine(s) — a composite exists to put looks \
                     side by side, and one tile is a comparison with nothing",
                    self.machines.len()
                ));
            }
            let mut seen: BTreeSet<u8> = BTreeSet::new();
            for &n in &self.machines {
                if !seen.insert(n) {
                    return Err(format!(
                        "gallery manifest: `{who}` names interpreter {n} twice — two tiles of one \
                         machine are identical by construction, which is the exact frame the guard \
                         exists to refuse"
                    ));
                }
                // A number with no MEASURED look has no period screen to show:
                // `zvm::interpreter` declines rather than guessing, and a tile of
                // a guess is worse than a missing tile.
                if zvm::interpreter::period_look_for(n, None).is_none() {
                    return Err(format!(
                        "gallery manifest: `{who}` names interpreter {n}, which `zvm::interpreter` \
                         has no measured period look for — there is no screen of that machine to put \
                         in a tile"
                    ));
                }
            }
            // The harness appends both to every tile's launch, so a shot that
            // also names one is either restating the truth or contradicting it —
            // and a contradicted `--colour` is six tiles of the reader's theme.
            for owned in ["--interpreter", "--colour", "--color"] {
                if self.args.iter().any(|a| a == owned) {
                    return Err(format!(
                        "gallery manifest: `{who}` is a composite passing `{owned}` — the harness \
                         appends `--interpreter N --colour machine` per tile, which is what makes \
                         the machine the only variable in the frame"
                    ));
                }
            }
        }
        // The three shapes where a prose floor can only ever read zero, and so
        // would fail every shot that set it — the same refusal, and for the same
        // reason, as `expect_art_cells` on half-blocks above.
        if self.expect_prose_cells > 0 {
            if matches!(self.v6_render, Some(app::config::V6RenderMode::Raster | app::config::V6RenderMode::Extended)) {
                return Err(format!(
                    "gallery manifest: `{who}` is a raster shot with `expect_prose_cells` — raster puts \
                     the whole screen in ONE IMAGE, so there is not a text cell on it to count. The \
                     guard a raster shot wants is `expect_art_cells`"
                ));
            }
            if self.pane_content_cells().is_none() {
                return Err(format!(
                    "gallery manifest: `{who}` sets `expect_prose_cells` on a shot whose story pane \
                     this file cannot locate — a map shot's pane is a percentage split resolved by \
                     ratatui, which is the app's arithmetic and not this file's to restate"
                ));
            }
        }
        if self.expect.is_empty() && self.expect_art_cells == 0 {
            return Err(format!(
                "gallery manifest: `{who}` sets neither `expect` nor `expect_art_cells` — a shot with \
                 no guard is a shot that cannot tell its own frame from a browser or a boot prompt"
            ));
        }
        // The tool owns these, and a manifest that sets them either fights the
        // backend choice or writes the gallery into the player's real home.
        // `--v6-pixel-lock` joins the list because the gallery now pins it ON for
        // every shot in the run's config (see `run_shot`). A shot passing it again
        // is at best a second copy of the truth and at worst `off`, which would be
        // one frame quietly softer than the rest.
        for owned in ["--image-protocol", "--user-dir", "--sound", "--v6-pixel-lock"] {
            if self.args.iter().any(|a| a == owned) {
                return Err(format!(
                    "gallery manifest: `{who}` passes `{owned}` — the gallery tool owns that argument \
                     (set `backend` instead of `--image-protocol`; the pixel lock is on for every \
                     shot already)"
                ));
            }
        }
        Ok(())
    }

    /// The terminal size in cells.
    pub fn size_cells(&self) -> Result<(u16, u16), String> {
        let (c, r) = self
            .size
            .split_once('x')
            .ok_or_else(|| format!("gallery manifest: `{}` has size `{}`, wanted COLSxROWS", self.id, self.size))?;
        let cols: u16 = c.trim().parse().map_err(|_| format!("gallery manifest: `{}`: bad column count in `{}`", self.id, self.size))?;
        let rows: u16 = r.trim().parse().map_err(|_| format!("gallery manifest: `{}`: bad row count in `{}`", self.id, self.size))?;
        if cols == 0 || rows == 0 {
            return Err(format!("gallery manifest: `{}` has a zero dimension in `{}`", self.id, self.size));
        }
        Ok((cols, rows))
    }

    /// The cell size in pixels this shot must be captured at — chosen by the
    /// BACKEND, never by the manifest.
    ///
    /// `Picker::halfblocks()` assumes a 10x20 cell whatever the terminal
    /// reported, so a half-block capture taken at any other size draws a
    /// geometry lanthorn was not using and every proportion in the picture is
    /// wrong. Kitty asks the terminal, so kitty gets the cell we answered
    /// `CSI 16 t` with — and what we answer is ours to choose well.
    ///
    /// BOTH CELLS ARE EXACTLY 1:2 (SQ-0963), and that is the point rather than a
    /// coincidence. A half-block sample is `cell_width` wide by `cell_height / 2`
    /// tall, so a square sample — equal resolution on both axes — wants a cell
    /// of exactly 1:2, and anything else samples the artwork finer across than
    /// down for no reason at all. `the_cell_is_square_for_half_block_samples`
    /// pins that ratio for both backends.
    ///
    /// THE KITTY CELL IS **16x32**, AND IT IS THE GAME'S OWN FONT (SQ-1001). It
    /// was 8x18, then 8x16, and 8x16 was half the size the frames needed. A v6
    /// press draws its text on an 8x16 GAME-pixel cell (`v6_layout::FONT_W` /
    /// `FONT_H`, `zvm::screen::V6_FONT_*`), hybrid mode gives each of those
    /// characters one TERMINAL cell, and the art beside it is magnified by `s` —
    /// so a terminal cell of `8 x 16` against art at `s = 2` renders type at half
    /// the size the game laid out. That is not a taste; it is visible in every
    /// frame taken before this quest, where Journey's menu is a third the height
    /// of its own box. The cell that matches is `8s x 16s`, and at the `s = 2`
    /// this manifest's standard grid uses that is 16x32: one game character to
    /// one terminal cell, exactly.
    ///
    /// The knock-on is that **a kitty shot may not magnify by less than 2**.
    /// At `s = 1` the game's own 80-column screen would have to fit 40 cells and
    /// its text overruns its windows — measured, on Journey at 42x16: "Individual
    /// Commands" came out as "Individual Comman". The 1x rung the pane-size row
    /// used to open on is gone for that reason and not for a nicer picture.
    ///
    /// 16x32 is also on the default face's exact-cell ladder: Fira Code's cell is
    /// 0.615/1.231 em = 2.000, so 26 px/em rounds to exactly 16x32 and
    /// [`Face::cell_complaint`] stays quiet (see [`FONT_CANDIDATES`]). 20x40 —
    /// the other size considered — does not: 32 px gives 20x39 and 33 px gives
    /// 20x41, so every shot would have been drawn in type that did not sit square
    /// in its cell.
    pub fn cell_px(&self) -> (u16, u16) {
        match self.backend {
            Backend::Kitty => (16, 32),
            Backend::Halfblocks => (10, 20),
        }
    }

    /// The story pane's CONTENT rect in cells — the box the v6 composite is
    /// magnified into — or `None` when this shot cannot answer.
    ///
    /// `compute_pane_layout` reserves one row for the help bar and nothing else
    /// while the command band and the inventory dock are closed (`layout.rs`),
    /// and `draw_framed` then insets the pane one cell on every side for its
    /// border. So a full-width story pane's content is `COLS - 2` by `ROWS - 3`.
    ///
    /// `None` for a map shot, deliberately. A split pane's width is a percentage
    /// of the frame resolved by ratatui, which is the app's arithmetic and not
    /// this file's to restate — and the one map shot in the manifest is a v3
    /// story with no pixel screen to magnify anyway.
    pub fn pane_content_cells(&self) -> Option<(u32, u32)> {
        if self.show_map {
            return None;
        }
        let (cols, rows) = self.size_cells().ok()?;
        Some((u32::from(cols).checked_sub(2)?, u32::from(rows).checked_sub(3)?))
    }

    /// How far the v6 composite is magnified in this shot: the aspect-preserving
    /// fit of `native` into the pane's device box, exactly as `uniform_scale`
    /// computes it (`v6_layout.rs`) — `min(box_w / native_w, box_h / native_h)`,
    /// unrounded and unclamped.
    ///
    /// WHY IT WANTS TO BE A WHOLE NUMBER (SQ-0963). At any other value every edge
    /// in the artwork is interpolated: the composite is resized once to
    /// `round(native * s)` and the bands are 1:1 crops out of that, so `s` is the
    /// only place softness can enter and a fractional `s` guarantees it. At an
    /// integer `s` one art pixel lands on a whole number of device pixels on both
    /// axes and the frame is exactly as crisp as the artwork is.
    ///
    /// This is a per-shot number and cannot be one constant: a Blorb press is
    /// 640x400, the standard Macintosh plate 480x304.
    pub fn magnification(&self, native: (u32, u32)) -> Option<f64> {
        let (cc, cr) = self.pane_content_cells()?;
        let (cw, ch) = self.cell_px();
        let (bw, bh) = (cc * u32::from(cw), cr * u32::from(ch));
        let (nw, nh) = (native.0.max(1), native.1.max(1));
        Some((f64::from(bw) / f64::from(nw)).min(f64::from(bh) / f64::from(nh)))
    }

    /// The scripted keys.
    pub fn keys(&self) -> Result<Vec<Key>, String> {
        self.keys
            .split(',')
            .filter(|t| !t.trim().is_empty())
            .map(|t| Key::parse(t).map_err(|e| format!("gallery manifest: `{}`: {e}", self.id)))
            .collect()
    }

    /// How many keypresses reached this frame — the turn count, counted off the
    /// spec rather than declared beside it.
    ///
    /// A frame is a fixture and this is half of its identity: Arthur's ProDOS
    /// press renders identically at 6 and 40 keypresses because it never answers
    /// the restore question, so "which frame is this" is unanswerable without it.
    pub fn turns(&self) -> usize {
        self.keys.split(',').filter(|t| {
            let t = t.trim();
            !t.is_empty() && !t.starts_with("wait:")
        }).count()
    }

    /// The arguments the tool adds on this shot's behalf, for the tile of
    /// `machine` — `None` for an ordinary single-frame shot.
    ///
    /// `--colour machine` travels with `--interpreter N` and is not optional
    /// (SQ-1154): the number alone tells the STORY which machine it is on, while
    /// the opt-in is what licenses lanthorn to paint that machine's screen on a
    /// bare story file. Without it the six tiles resolve their page and ink
    /// through the reader's theme and come out identical — which is precisely
    /// the frame [`check_machines_differ`] refuses.
    pub fn lanthorn_args_for(&self, machine: Option<u8>) -> Vec<String> {
        let mut v = self.args.clone();
        if self.backend == Backend::Halfblocks {
            v.push("--image-protocol".into());
            v.push("halfblocks".into());
        }
        if let Some(n) = machine {
            v.push("--interpreter".into());
            v.push(n.to_string());
            v.push("--colour".into());
            v.push("machine".into());
        }
        v
    }

    /// The arguments for a shot with no machine of its own.
    pub fn lanthorn_args(&self) -> Vec<String> {
        self.lanthorn_args_for(None)
    }

    /// One entry per frame this shot captures: `[None]` for an ordinary shot,
    /// one `Some(n)` per tile for a composite.
    ///
    /// A value rather than a bool and a list, so no caller can ask for the tiles
    /// and forget that a plain shot still has exactly one run (CLAUDE.md's
    /// refactoring policy — the parameter list that lets a caller supply half the
    /// subject is how this file has been bitten twice already).
    pub fn runs(&self) -> Vec<Option<u8>> {
        if self.machines.is_empty() {
            vec![None]
        } else {
            self.machines.iter().map(|&n| Some(n)).collect()
        }
    }

    /// The throwaway user directory, and the progress line, for one run.
    pub fn run_id(&self, machine: Option<u8>) -> String {
        match machine {
            Some(n) => format!("{}-i{n}", self.id),
            None => self.id.clone(),
        }
    }

    /// How many columns the tiles are laid out in: the count that makes the
    /// finished PICTURE closest to square.
    ///
    /// **The squarest grid is not the squarest picture, and that is the whole of
    /// this.** It was `ceil(sqrt(n))` — six tiles as 3x2 — which reads as the
    /// obvious answer and produced a 3128x1402 frame, wider than 2:1. A picture
    /// that wide is scaled down by whatever is showing it until the prose in it
    /// cannot be read, and the prose is the entire subject of a shot about how six
    /// machines paint text. The step being skipped is that a TILE is a terminal
    /// and already landscape — 64x20 cells on a 16x32 cell is 1024x640, 1.6:1 —
    /// so laying wide things out wide multiplies it.
    ///
    /// The question is asked about the output instead: for each candidate column
    /// count the finished picture is `c` tiles across by `ceil(n / c)` down,
    /// gutters included since they are what actually gets written, and the one
    /// whose aspect is nearest 1:1 wins. Measured on this manifest's composite,
    /// 3 columns is 2.44:1 and **2 columns is 1.09:1**, at 2090x1922 — a
    /// window-shaped picture instead of a banner.
    ///
    /// Compared in LOG space, so 2:1 and 1:2 are the same distance from square. A
    /// plain ratio would rate every portrait candidate nearer than every landscape
    /// one and always answer 1.
    pub fn tile_columns(&self) -> usize {
        let n = self.machines.len().max(1);
        let Ok((cols, rows)) = self.size_cells() else { return 1 };
        let (cell_w, cell_h) = self.cell_px();
        let tile_w = f64::from(u32::from(cols) * u32::from(cell_w) + TILE_GUTTER);
        let tile_h = f64::from(u32::from(rows) * u32::from(cell_h) + TILE_GUTTER);
        let squareness = |c: usize| {
            ((c as f64 * tile_w) / (n.div_ceil(c) as f64 * tile_h)).ln().abs()
        };
        (1..=n).min_by(|&a, &b| squareness(a).total_cmp(&squareness(b))).unwrap_or(1)
    }

    /// The medium's absolute path.
    pub fn media_path(&self) -> PathBuf {
        repo_root().join(&self.media)
    }

    /// The archive this shot names with `--pictures`, if it names one.
    ///
    /// Read back out of `args` rather than declared a second time. It is not
    /// decoration: a named archive picks BOTH the artwork and the machine — a
    /// DOS `.eg1` asks for the IBM PC, the Macintosh's monochrome `Pic.data` for
    /// a two-colour Macintosh — and it changes the picture SPACE the press lays
    /// itself out on, which is the denominator of every magnification this file
    /// computes. A [`Provenance`] read without it describes a screen the shot
    /// never booted, and every number around it stays self-consistently wrong
    /// (SQ-1001).
    pub fn pictures(&self) -> Option<&str> {
        let i = self.args.iter().position(|a| a == "--pictures")?;
        self.args.get(i + 1).map(String::as_str)
    }
}

// ── Provenance ────────────────────────────────────────────────────────────────

/// What the launch turned out to be pointed at, read at capture time.
///
/// TWO ARMS, BECAUSE A DIRECTORY IS NOT A FILE (SQ-1080). A story shot's
/// provenance is measured and never declared: `load_mounted_story` mounts the
/// file the way the app does and the release and serial come out of the header
/// of the bytes it returned. A disk image is a different BUILD of the game, not
/// the same story on other media, so a caption that names a release it did not
/// load is worse than one that names none.
///
/// A LIBRARY launch has none of those facts and cannot be made to have them.
/// Nothing has been mounted: the picker is a list of candidates, and which of
/// them a player would have opened is the one thing the frame does not say. The
/// honest provenance for that shot is the DIRECTORY, and this type says so with
/// its own arm rather than by filling a story's fields with plausible zeroes —
/// which would put `v0 r0/s000000` under a frame, or worse, the header of
/// whichever file the directory happened to sort first.
///
/// The count is deliberately not here either. The picker prints its own
/// (`105 found in stories`), that number is the scan's after containers are
/// expanded and duplicate builds folded, and a second count computed a different
/// way beside it would be exactly the "second copy of the truth" this file's
/// header warns about. `expect` reads it off the frame instead.
#[derive(Clone, Debug)]
pub enum Provenance {
    /// One story, mounted.
    Story(StoryProvenance),
    /// A directory of stories, none of them opened: the picker IS the frame.
    Library {
        /// The library's manifest id.
        id: String,
        /// Where its members came from, relative to the repo root.
        from: String,
        /// How many of them were staged.
        members: usize,
    },
}

/// Everything the mounted bytes of ONE story say about themselves.
#[derive(Clone, Debug)]
pub struct StoryProvenance {
    pub version: u8,
    pub release: u16,
    pub serial: String,
    /// The filesystem the mount reported, in prose, or "story file".
    pub medium: String,
    /// The v6 native screen in zvm pixels — the size the art is magnified FROM.
    /// `None` for every non-v6 story, which has no pixel screen to speak of.
    ///
    /// Derived like everything else here: this press's own picture space at this
    /// press's own art scale. It is not one number for the corpus — a Blorb press
    /// is 640x400, the standard Macintosh plate is 480x304, Arthur's Apple II
    /// press is 560x384 — so the pane size that magnifies it by a whole number
    /// is a per-shot answer and not a constant (SQ-0963).
    pub native: Option<(u32, u32)>,
}

impl Provenance {
    /// The provenance of whatever this shot points at — a mounted story, or the
    /// library a library shot opens the picker on.
    pub fn of(subject: &Subject<'_>, pictures: Option<&str>) -> Result<Provenance, String> {
        match subject {
            Subject::Medium(path) => Provenance::read(path, pictures).map(Provenance::Story),
            Subject::Library(l) => Ok(Provenance::Library {
                id: l.id.clone(),
                from: l.from.clone(),
                members: l.members.len(),
            }),
        }
    }

    /// The v6 native screen, for a story that has one.
    pub fn native(&self) -> Option<(u32, u32)> {
        match self {
            Provenance::Story(s) => s.native,
            Provenance::Library { .. } => None,
        }
    }

    /// `pictures` is the shot's `--pictures` name, from [`Shot::pictures`], and
    /// has to be passed for the same reason lanthorn itself resolves the
    /// override before it builds the engine: the named archive settles both the
    /// interpreter profile and the picture space, so a native screen read
    /// without it is the DEFAULT rendition's screen wearing this shot's caption.
    /// Zork Zero's Macintosh disk is the case that shows it — `CPic.data` is
    /// 320x200 doubled to 640x400, its monochrome `Pic.data` is 480x300 at 1:1,
    /// and the two want different pane sizes to magnify by a whole number.
    pub fn read(path: &Path, pictures: Option<&str>) -> Result<StoryProvenance, String> {
        let (loaded, image) = app::hints::load_mounted_story(path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let bytes = loaded.bytes();
        if bytes.len() < 0x18 {
            return Err(format!("{}: too short to carry a Z-machine header", path.display()));
        }
        // A NAMED ARCHIVE THAT DOES NOT LOAD IS A REFUSAL, not a fallback. In the
        // app it is: lanthorn says so and draws the Blorb instead, which is the
        // right call for a player mid-launch. In a gallery it would be a shot
        // captioned "the monochrome plates" showing the colour ones, with the
        // release, serial and medium all still perfectly correct — the exact
        // shape of failure `expect` exists for, and `expect` cannot see it
        // because both renditions draw the same scene.
        //
        // Only "did not load" is fatal. `warning()` also speaks for a LOADED
        // archive whose continuation volume was refused — Arthur's and Journey's
        // EGA ship as `.EG1` + `.EG2` — and that is a frame with fewer pictures
        // in it, not a frame of the wrong rendition.
        let over = picture_override(path, pictures);
        if matches!(
            over,
            app::graphics::PictureOverride::Missing { .. } | app::graphics::PictureOverride::Unusable { .. }
        ) {
            return Err(over.warning().unwrap_or_else(|| "the named archive did not load".into()));
        }
        Ok(StoryProvenance {
            version: bytes[0],
            release: u16::from_be_bytes([bytes[2], bytes[3]]),
            serial: String::from_utf8_lossy(&bytes[0x12..0x18]).into_owned(),
            medium: medium_name(image),
            native: (bytes[0] == 6).then(|| native_screen(path, image, over)).flatten(),
        })
    }

    /// `v6 r83/s890706 off a story file`, or — for a library launch — the
    /// directory and the fact that nothing in it has been opened.
    pub fn describe(&self) -> String {
        match self {
            Provenance::Story(s) => s.describe(),
            Provenance::Library { id, from, members } => {
                format!("the `{id}` library — {members} stories staged from {from}/, none opened")
            }
        }
    }
}

impl StoryProvenance {
    /// `v6 r83/s890706 off a story file`.
    pub fn describe(&self) -> String {
        format!("v{} r{}/s{} off {}", self.version, self.release, self.serial, self.medium)
    }
}

/// Resolve a shot's `--pictures` name the way lanthorn's own launch does.
///
/// Only the shot's own flag reaches here. The per-game sidecar the two-argument
/// [`app::graphics::PictureOverride::resolve`] would also read belongs to a save
/// directory the gallery creates fresh for every capture, so there is never one
/// to find — and looking for one anyway would let a stale sidecar on this machine
/// change what the committed manifest captures.
fn picture_override(path: &Path, pictures: Option<&str>) -> app::graphics::PictureOverride {
    match pictures {
        Some(name) => app::graphics::PictureOverride::resolve_with_session(path, path, Some(name)),
        None => app::graphics::PictureOverride::Unset,
    }
}

/// The v6 native screen this press lays itself out on, in zvm pixels.
///
/// The chain is `startup.rs`'s and `session.rs`'s, written out rather than
/// approximated: the picture space through `std_window → native_std_window →
/// profile`, times the art scale that space is drawn at. CLAUDE.md is emphatic
/// that a harness which skips a rung of it measures a screen the player never
/// sees — Journey r77 and Arthur r63 are 560x384 presses that come out 640x400
/// if `native_std_window` is left off — and this number is the DENOMINATOR of
/// every magnification below, so getting it wrong would make each of them
/// self-consistently wrong.
///
/// `over` is the shot's `--pictures` archive, already resolved, and enters the
/// chain exactly where `startup.rs` puts it: the override is settled first, its
/// FLAVOUR selects the profile, and the loaded archive outranks the Blorb and the
/// medium's own art.
fn native_screen(
    path: &Path,
    image: Option<app::hints::DiskImage>,
    over: app::graphics::PictureOverride,
) -> Option<(u32, u32)> {
    let flavour = over.flavour();
    let profile = app::interpreter::InterpreterProfile::resolve(path, None, flavour, image);
    let picts = app::graphics::PictSource::resolve_with_override(path, over, None);
    let space = picts.std_window().or_else(|| picts.native_std_window()).or_else(|| profile.std_window());
    let art_scale = picts.art_scale();
    // `session.rs`'s own rule: a declared picture space is drawn at the scale
    // this machine drew it; absent one there is nothing to scale and the
    // uniform doubling stands.
    let (aw, ah) = space.unwrap_or((320, 200));
    let (sx, sy) = match (space, art_scale) {
        (Some(_), Some(s)) => s,
        _ => (2, 2),
    };
    Some((u32::from(aw) * sx.max(1), u32::from(ah) * sy.max(1)))
}

fn medium_name(image: Option<app::hints::DiskImage>) -> String {
    use app::hints::DiskImage as D;
    match image {
        Some(D::Adf) => "an Amiga floppy",
        Some(D::Hfs) => "a Macintosh floppy",
        Some(D::Fat12Dos) => "a DOS floppy",
        Some(D::Fat12AtariSt) => "an Atari ST floppy",
        Some(D::ProDos) => "an Apple ProDOS floppy",
        Some(D::InfocomBootDisk) => "an Apple self-booting floppy",
        Some(D::CommodoreD64) => "a Commodore 1541 floppy",
        Some(D::CommodoreG64) => "a Commodore 1541 floppy, nibbled to GCR",
        Some(D::Iso9660) => "an ISO 9660 CD-ROM",
        None => "a story file",
    }
    .to_string()
}

// ── Capturing one shot ────────────────────────────────────────────────────────

/// Everything a finished shot knows about itself — the record that goes into
/// `gallery.json` and under the picture.
#[derive(Clone, Debug)]
pub struct Taken {
    pub id: String,
    pub png: PathBuf,
    pub provenance: Provenance,
    pub cols: u16,
    pub rows: u16,
    pub cell_w: u16,
    pub cell_h: u16,
    pub turns: usize,
    pub seed: u32,
    pub backend: Backend,
    /// The face the glyphs were drawn with, named so a reader knows the type in
    /// the picture is the harness's and not a terminal's.
    pub face: String,
    /// The driver's own verdict on which protocol negotiated.
    pub verdict: String,
    /// How many boots it took to reach a frame that passed the shot's guard.
    /// More than one is worth seeing: it means this shot is timing-sensitive.
    pub attempts: usize,
    pub captured_bytes: usize,
    pub width: u32,
    pub height: u32,
    /// The v6 native screen this press lays out on, and how far the pane
    /// magnified it. `None` for a story with no pixel screen, and for the map
    /// shot whose pane is a split this file does not restate.
    pub native: Option<(u32, u32)>,
    pub magnification: Option<f64>,
    /// Characters neither the face nor the bitmap master could draw.
    pub unresolved_glyphs: Vec<char>,
    /// The machines this frame tiles, in the order they were laid out. Empty for
    /// every ordinary shot — a composite is the only kind with more than one
    /// launch behind one picture (SQ-1165).
    pub machines: Vec<u8>,
}

impl Taken {
    /// `640x400 native, 2.000x` — or a complaint when the magnification is not a
    /// whole number, since that is the one thing about it worth reading.
    pub fn scale_note(&self) -> Option<String> {
        let (n, m) = (self.native?, self.magnification?);
        let whole = (m - m.round()).abs() < 1e-9 && m >= 1.0;
        Some(format!(
            "{}x{} native at {m:.3}x{}",
            n.0,
            n.1,
            if whole { "" } else { " (NOT a whole number — every edge in the art is interpolated)" }
        ))
    }
}

/// Write the settings this run captures under, into the shot's own throwaway user
/// directory: the pinned seed, the pixel lock, the render mode, the patched-font
/// icon answer, and — for a library shot — the default story directory.
///
/// **This is where a setting that should hold for the WHOLE gallery goes.** A shot
/// can still ask for something of its own through `args`, but anything the set
/// should agree on belongs here, so that adding a row to `gallery.toml` inherits it
/// instead of having to remember it. The seam already existed for `random_seed` and
/// `default_story_dir`; SQ-1152 put the pixel lock and the font-check answer
/// through it too.
///
/// Split out of `capture` so it can be tested without a pty: everything below is
/// text on disk, and a case can read it back.
pub fn write_run_settings(user_dir: &Path, shot: &Shot, media: &Path) -> Result<(), String> {
    // The seed goes in the global config rather than the per-game sidecar: the
    // sidecar is a bare-lines file the driver already owns for `show_map`, and
    // two writers of one file is how a shot silently loses its seed.
    // `v6_render_key` rather than a literal, so the manifest's token and the one
    // the app parses back cannot drift apart (the same reason SQ-1079 gave it).
    let render = shot.v6_render.map_or(String::new(), |m| {
        format!("v6_render = \"{}\"\n", app::config::v6_render_key(m))
    });
    // A LIBRARY LAUNCH ASKS A QUESTION BEFORE THE TERMINAL EXISTS (SQ-1080).
    // `resolve_launch` offers to remember a directory passed on the command line
    // as `default_story_dir`, and `prompt_yes_no` reads a LINE from stdin in
    // cooked mode — before the colour query, before raw mode, before anything is
    // drawn. Nothing a key spec can send answers it usefully: the driver's keys
    // start after the app has settled, and a `cr` typed into that prompt would
    // then be a `cr` the picker never receives.
    //
    // So it is not answered; it is not ASKED. The prompt fires only when the
    // config has no `default_story_dir`, and this run's config is ours to write
    // — the same trick `pty_query_replies::library` uses, and for the same
    // reason. A fresh user dir per shot means the answer cannot leak between
    // shots or into anyone's real `~/.lanthorn`.
    let default_dir = if shot.library {
        // `to_string_lossy` and TOML's literal string: the path is this
        // repository's own and carries no quote to escape.
        format!("default_story_dir = '{}'\n", media.display())
    } else {
        String::new()
    };
    std::fs::write(
        user_dir.join("config.toml"),
        format!("random_seed = {}\nv6_pixel_lock = true\n{render}{default_dir}", shot.seed),
    )
    .map_err(|e| format!("writing the pinned seed: {e}"))?;
    // EVERY v6 SHOT IS PIXEL-LOCKED, and it is set HERE rather than per shot so
    // that the next one added gets it without anyone having to remember.
    //
    // It used to be `--v6-pixel-lock on` in two shots' `args`, which made it look
    // like a property of those two frames. It is a property of the gallery: a
    // fractional magnification puts an art pixel on a fractional number of device
    // pixels, and every edge in the frame is then interpolated. `--v6-pixel-lock`
    // is on the tool-owned list in `validate` for the same reason
    // `--image-protocol` is — a shot that turned it off would be fighting this.
    //
    // It costs the existing frames nothing, and that is checkable rather than
    // hopeful: `every_v6_shot_magnifies_by_a_whole_number` already fails any shot
    // whose FREE scale is fractional, and a whole number is always a valid rung, so
    // on a correctly-sized shot the lock has nothing to snap to. It is a floor under
    // the sizes, not a change to them. On the modes and backends where it does not
    // apply it is inert by construction — `v6_pixel_lock_applies` gates it off for
    // half-blocks (SQ-0978), a non-v6 story has no rung ladder at all, and
    // `extended` already pins a strictly finer whole-NATIVE-pixel magnification of
    // its own (SQ-1032), so the key is harmless where it is not the thing deciding.
    //
    // AND EVERY SHOT DRAWS THE PATCHED-FONT ICONS. The frames are captured through a
    // Nerd Font (see `FONT_CANDIDATES`), so the plain Geometric Shapes fallback is
    // the wrong half of a choice the reader's terminal has already made for them —
    // `zork1-map` reported `NO GLYPH ANYWHERE FOR: ◈◌` for exactly that reason.
    //
    // This calls the APP'S OWN writer with the answer a "yes" to the font check
    // gives, rather than spelling the preset names here: `write_font_check_answer`
    // is what `/run-font-check` writes, so the gallery cannot drift from it, and a
    // later improvement to the `nerdfont` presets reaches these frames for free.
    // That is also why the preset NAMES are written and not forty expanded
    // codepoints — the same argument that function's own doc makes.
    //
    // What it does NOT set, so nobody reads the absence as an oversight: the six
    // `badge_*` glyphs. They are `[elements]` keys with no `nerdfont` preset behind
    // them — the font check has never touched them — so putting Nerd codepoints in
    // the picker's badges is a decision about the APP's presets, not about this
    // harness, and freezing six literals here is precisely what the paragraph above
    // says not to do.
    let style_path = app::style::personal_style_path(user_dir);
    app::style::write_font_check_answer(&style_path, true)
        .map_err(|e| format!("writing the font-check answer: {e}"))?;
    Ok(())
}

/// Boot lanthorn for one shot and hand back the capture, having first refused
/// every way the capture could be of the wrong thing.
///
/// `machine` is the §11.1.3 interpreter number this TILE is of, for a composite
/// shot, and `None` for every other row — see [`Shot::runs`].
pub fn capture(
    shot: &Shot,
    machine: Option<u8>,
    subject: &Subject<'_>,
    bin: &Path,
    work: &Path,
    timeout: std::time::Duration,
) -> Result<Capture, String> {
    // The path lanthorn is launched with: the medium itself, or the directory
    // this shot's library was just staged into.
    let media = match subject {
        Subject::Medium(path) => {
            if !path.exists() {
                return Err(format!(
                    "`{}`: no medium at {} (the media directories are gitignored)",
                    shot.id,
                    path.display()
                ));
            }
            path.clone()
        }
        Subject::Library(l) => l.stage(work).map_err(|e| format!("`{}`: {e}", shot.id))?,
    };
    let (cols, rows) = shot.size_cells()?;
    let (cell_w, cell_h) = shot.cell_px();

    // ONE USER DIRECTORY PER TILE, not per shot. Every per-game sidecar, saved
    // colour answer and font-check reply a run writes lands in here, and a
    // composite boots the SAME story six times: a shared directory would let the
    // first machine's sidecar decide what the sixth one launches with, which is
    // the frame's one variable leaking between its own tiles.
    let user_dir = work.join(shot.run_id(machine));
    let _ = std::fs::remove_dir_all(&user_dir);
    std::fs::create_dir_all(&user_dir).map_err(|e| format!("`{}`: {e}", shot.id))?;
    write_run_settings(&user_dir, shot, &media).map_err(|e| format!("`{}`: {e}", shot.id))?;

    let mut spec = Spec::new(bin, &media, &user_dir);
    // THE PICKER PRINTS THE PATH IT WAS GIVEN, so for a library shot the path is
    // part of the picture. Launch from the staged directory's parent and name the
    // directory — `lanthorn frontispieces` — which is the launch a person makes
    // and the one the header can hold. Named absolutely it is 60 characters of
    // system temp directory that clip the key hints off the end of the header.
    if let (Subject::Library(_), Some(parent), Some(name)) =
        (subject, media.parent(), media.file_name())
    {
        spec.cwd = Some(parent.to_path_buf());
        spec.story = PathBuf::from(name);
    }
    spec.cols = cols;
    spec.rows = rows;
    spec.cell_w = cell_w;
    spec.cell_h = cell_h;
    // The per-game sidecar `hide_map` writes is keyed on the STORY path, and a
    // library shot's path is a directory — so there is no game to write one for.
    // (`validate` has already refused `show_map` on a library shot, so this is
    // the whole of the difference.)
    spec.hide_map = !shot.show_map && !shot.library;
    spec.keys = shot.keys()?;
    spec.timeout = timeout;
    spec.extra_args = shot.lanthorn_args_for(machine);

    let cap = driver::run(spec).map_err(|e| format!("`{}`: {e}", shot.id))?;
    // A run cut short is a frame captured mid-script, and it looks exactly like
    // a frame captured on purpose — the keys the ceiling ate leave no mark on
    // the picture. Refuse it here rather than let the guard downstream report a
    // frame that "does not say" something the shot never got far enough to say.
    if cap.timed_out {
        return Err(format!(
            "`{}`: hit the {}s ceiling with keys still unsent — raise --timeout, or the key spec's \
             waits add up to more than it allows",
            shot.id,
            timeout.as_secs()
        ));
    }
    let neg = cap.negotiated();
    match shot.backend {
        // Not a warning. A capture that fell back measures a renderer the shot
        // did not ask for, and a gallery of the wrong renderer is worse than a
        // gallery with a hole in it.
        Backend::Kitty if !neg.is_kitty() => Err(format!("`{}`: {}", shot.id, neg.explain())),
        // The mirror of it: `--image-protocol halfblocks` was passed, so any APC
        // graphics at all means the flag did not take and this is not the frame
        // the manifest asked for.
        Backend::Halfblocks if neg.apc_commands > 0 => Err(format!(
            "`{}`: asked for half-blocks and got {} APC `_G` command(s) — the backend override did not take",
            shot.id, neg.apc_commands
        )),
        _ => Ok(cap),
    }
}

/// Every cell of the resolved screen as text, one line per row.
///
/// The kitty unicode placeholder is a cell carrying an image rather than a
/// glyph, so it reads as a space: an art cell is not text and must not look like
/// some to a substring search.
pub fn screen_text(res: &super::oracle::Resolved) -> String {
    let mut s = String::with_capacity(usize::from(res.rows) * (usize::from(res.cols) + 1));
    for row in 0..res.rows {
        for col in 0..res.cols {
            let ch = res.cell(row, col).ch;
            s.push(if matches!(ch, '\0' | '\u{10EEEE}') { ' ' } else { ch });
        }
        s.push('\n');
    }
    s
}

/// How many cells a placement would actually put pixels on.
pub fn art_cells(res: &super::oracle::Resolved) -> usize {
    let mut n = 0;
    for row in 0..res.rows {
        for col in 0..res.cols {
            if res.cell(row, col).image_id.is_some() {
                n += 1;
            }
        }
    }
    n
}

/// How many cells of the STORY PANE carry a letter or a digit the game wrote.
///
/// `None` when the shot cannot say where its pane is (see
/// [`Shot::pane_content_cells`]), which [`Shot::validate`] has already refused
/// for any shot that sets a floor.
///
/// The pane's content rect is rows `1..=ROWS-3` by columns `1..=COLS-2`: one row
/// off the bottom for the help bar, and one cell of border on every side. That
/// inset is the point rather than an implementation detail — the header outside it
/// names the story and the medium, and the help bar under it lists three key
/// bindings, so a count taken over the whole screen never reaches zero and can
/// never fail.
pub fn prose_cells(shot: &Shot, res: &super::oracle::Resolved) -> Option<usize> {
    let (cols, rows) = shot.pane_content_cells()?;
    let mut n = 0;
    for row in 1..=u16::try_from(rows).ok()?.min(res.rows.saturating_sub(1)) {
        for col in 1..=u16::try_from(cols).ok()?.min(res.cols.saturating_sub(1)) {
            let cell = res.cell(row, col);
            // A cell under a placement is ARTWORK, whatever glyph the placeholder
            // encoding left in it, and must not read as text.
            if cell.image_id.is_none() && cell.ch.is_alphanumeric() {
                n += 1;
            }
        }
    }
    Some(n)
}

/// The non-vacuity guard: everything the shot said must be on screen, is.
///
/// A failure prints what IS on the screen, because "not the frame you asked for"
/// is only actionable next to the frame you got — and the fix is nearly always
/// another keypress rather than a weaker guard.
pub fn check_expectations(shot: &Shot, res: &super::oracle::Resolved) -> Result<(), String> {
    let text = screen_text(res);
    let missing: Vec<&str> = shot.expect.iter().map(|s| s.as_str()).filter(|w| !text.contains(*w)).collect();
    let art = art_cells(res);
    let prose = prose_cells(shot, res).unwrap_or(0);
    if missing.is_empty() && art >= shot.expect_art_cells && prose >= shot.expect_prose_cells {
        return Ok(());
    }
    let mut why: Vec<String> = Vec::new();
    if !missing.is_empty() {
        why.push(format!(
            "does not say {}",
            missing.iter().map(|m| format!("{m:?}")).collect::<Vec<_>>().join(" or ")
        ));
    }
    if art < shot.expect_art_cells {
        why.push(format!("puts art on {art} cell(s), wanted at least {}", shot.expect_art_cells));
    }
    if prose < shot.expect_prose_cells {
        why.push(format!(
            "writes {prose} letter(s) into the story pane, wanted at least {} — the frame reached a \
             menu or a card and the game has not narrated yet",
            shot.expect_prose_cells
        ));
    }
    let seen: Vec<String> = text
        .lines()
        .map(|l| l.trim_end().to_string())
        .filter(|l| !l.trim().is_empty())
        .take(12)
        .collect();
    Err(format!(
        "`{}`: the frame {} — this is not the screen the manifest asked for (a browser, a boot \
         prompt, or a different story off the same medium). What it does say, {} art cell(s) aside:\n{}",
        shot.id,
        why.join(", and "),
        art,
        if seen.is_empty() { "        (no text at all — an all-art frame)".to_string() } else { seen.iter().map(|l| format!("        {l}")).collect::<Vec<_>>().join("\n") }
    ))
}

// ── The composite guard (SQ-1165) ─────────────────────────────────────────────

/// The colours ONE tile came out with, read off its resolved story pane.
///
/// Three readings rather than one, because the frame's subject is spread across
/// two surfaces. [`Self::page`] and [`Self::ink`] are the body — the ground the
/// prose sits on and the colour it is written in — and they are what separates
/// the Macintosh's black-on-white from the Apple's white-on-black. [`Self::pairs`]
/// is every distinct pair in the pane, which is where the STATUS LINE shows: a
/// reverse-video band states no new colour at all, so a comparison that only
/// looked at the dominant pair would be blind to the one surface a Version 3
/// story was chosen for.
///
/// Colours are kept in the form the stream expressed them ([`decode::Color`]),
/// never collapsed to RGB. A palette index and a truecolour triple that happen to
/// name the same shade are different statements by lanthorn, and folding them
/// together here would let a tile that stopped sending its machine's colour pass
/// as one that still did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaneLook {
    /// The most common background over the pane's content cells.
    pub page: super::decode::Color,
    /// The most common foreground among the cells carrying a letter or a digit.
    pub ink: super::decode::Color,
    /// Every distinct `(background, foreground)` pair in the pane.
    pub pairs: BTreeSet<(super::decode::Color, super::decode::Color)>,
}

impl PaneLook {
    /// `page #000000 ink #55ffff, 3 pair(s)` — what a failure needs to print.
    pub fn describe(&self) -> String {
        format!("page {} ink {}, {} pair(s)", self.page.label(), self.ink.label(), self.pairs.len())
    }
}

/// Read one tile's [`PaneLook`] off the resolved screen, or `None` when the shot
/// cannot say where its pane is (see [`Shot::pane_content_cells`]).
///
/// The rect is [`prose_cells`]', for the same reason: lanthorn's own header and
/// help bar are painted in the THEME's colours whatever the machine did, so a
/// screen-wide reading is dominated by cells no machine can move, and two tiles
/// would compare equal while their story panes differed completely.
///
/// SGR 7 is undone rather than carried, so a reversed status band is read as the
/// pair it puts on screen. That is the only way the band can enter [`Self::pairs`]
/// at all — a reverse states no colour of its own.
pub fn pane_look(shot: &Shot, res: &super::oracle::Resolved) -> Option<PaneLook> {
    use std::collections::BTreeMap;

    let (cols, rows) = shot.pane_content_cells()?;
    let last_row = u16::try_from(rows).ok()?.min(res.rows.saturating_sub(1));
    let last_col = u16::try_from(cols).ok()?.min(res.cols.saturating_sub(1));
    let mut grounds: BTreeMap<super::decode::Color, usize> = BTreeMap::new();
    let mut inks: BTreeMap<super::decode::Color, usize> = BTreeMap::new();
    let mut pairs: BTreeSet<(super::decode::Color, super::decode::Color)> = BTreeSet::new();
    for row in 1..=last_row {
        for col in 1..=last_col {
            let cell = res.cell(row, col);
            // A cell under a placement is artwork and states nothing about the
            // machine's text colours — the same exclusion `prose_cells` makes.
            if cell.image_id.is_some() {
                continue;
            }
            let (bg, fg) = if cell.inverse { (cell.fg, cell.bg) } else { (cell.bg, cell.fg) };
            *grounds.entry(bg).or_default() += 1;
            pairs.insert((bg, fg));
            if cell.ch.is_alphanumeric() {
                *inks.entry(fg).or_default() += 1;
            }
        }
    }
    // Ties broken on the colour itself, so one frame cannot read two ways.
    let pick = |m: BTreeMap<super::decode::Color, usize>| {
        m.into_iter().max_by_key(|(c, n)| (*n, *c)).map(|(c, _)| c)
    };
    Some(PaneLook { page: pick(grounds)?, ink: pick(inks)?, pairs })
}

/// The body a machine's MEASURED look states: page, ink, and how the status line
/// was set apart.
///
/// A type rather than a tuple because the three are one subject and are always
/// asked together — and because the CARET is deliberately outside it. A caret is
/// a shape, not a colour, and a rendered tile cannot be compared on one; two
/// machines that share a body and differ only in caret are twins as far as
/// [`check_machines_differ`] can see, which is exactly what interpreters 2 and 8
/// are.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MeasuredBody {
    pub page: (u8, u8, u8),
    pub ink: (u8, u8, u8),
    pub status: zvm::interpreter::StatusBand,
}

impl MeasuredBody {
    /// `#000000 under #55ffff` — the pair, as a failure needs to print it.
    fn describe(&self) -> String {
        format!(
            "#{:02x}{:02x}{:02x} under #{:02x}{:02x}{:02x}",
            self.page.0, self.page.1, self.page.2, self.ink.0, self.ink.1, self.ink.2
        )
    }
}

/// What `zvm::interpreter` measured for this machine at this story's Version, or
/// `None` for a machine it declines to guess at.
fn measured_body(n: u8, zversion: u8) -> Option<MeasuredBody> {
    let l = zvm::interpreter::period_look_for(n, Some(zversion))?;
    Some(MeasuredBody { page: l.page, ink: l.ink, status: l.status })
}

/// THE GUARD THIS KIND OF SHOT EXISTS FOR: two machines whose screens were
/// measured as different must not have come out the same (SQ-1165).
///
/// # Why `expect` cannot ask this
///
/// A composite's whole subject is the DIFFERENCE between its tiles, and every
/// tile renders the same story at the same moment — so every string `expect`
/// could name is on all six of them, and six copies of one palette pass it
/// unanimously. That is SQ-1164's failure exactly, one shape along: five Journey
/// frames named headings that were on the wrong screen and shipped for months
/// with an empty story pane, because the guard asked a question the defect could
/// answer. `expect_prose_cells` was the fix there and this is the fix here — a
/// floor on the thing the frame is actually about.
///
/// It doubles as a real test of SQ-1154: `--colour machine` on a bare `.z3` is
/// what licenses the machine's page and ink, and if that licence ever stops
/// reaching a raw file every tile falls through to the reader's theme and this
/// fails on the first pair it looks at.
///
/// # Why the requirement is derived and not a list
///
/// The obligation is read out of `zvm::interpreter` per pair rather than written
/// down here, because **not every pair of machines has a different screen**, and
/// a hand-written "these must differ" list would be a second copy of a table that
/// is already measured. Interpreter 2 (Apple) and interpreter 8 (Commodore 64)
/// are both white on black with a full-width reverse band: they differ only in
/// caret shape, `CursorShape::Block` against `CursorShape::Underscore`, which is
/// a glyph and not a colour. Demanding a colour difference there would fail a
/// frame that is perfectly correct. So [`measured_body`] is the question — page,
/// ink, band — and the caret is left out of it on purpose.
///
/// `zversion` is the story's header byte 0, and it is load-bearing rather than
/// tidy: the IBM PC's row stores no RGB at all and resolves its pair through the
/// palette its VERSION picks, which is XZIP's `#AAAAAA` white below Version 6 and
/// YZIP's `#FFFFFF` at it.
pub fn check_machines_differ(shot: &Shot, zversion: u8, tiles: &[(u8, PaneLook)]) -> Result<(), String> {
    let mut same: Vec<String> = Vec::new();
    for (i, (a, look_a)) in tiles.iter().enumerate() {
        for (b, look_b) in tiles.iter().skip(i + 1) {
            let (Some(ma), Some(mb)) = (measured_body(*a, zversion), measured_body(*b, zversion)) else {
                continue;
            };
            if ma == mb || look_a != look_b {
                continue;
            }
            same.push(format!(
                "interpreter {a} ({}) and interpreter {b} ({}) both rendered {} — but their screens \
                 were measured apart: {} against {}",
                machine_name(*a),
                machine_name(*b),
                look_a.describe(),
                ma.describe(),
                mb.describe(),
            ));
        }
    }
    if same.is_empty() {
        return Ok(());
    }
    Err(format!(
        "`{}`: the tiles do not differ, which is the only thing this frame claims:\n{}\n        \
         Every tile draws the same story at the same moment, so `expect` passes on all of them \
         whatever colour they came out — this is the guard that can tell. The usual cause is the \
         machine-colour LICENCE not reaching a bare story file: a tile launches \
         `--interpreter N --colour machine`, and without the second half of that pair the page and \
         ink resolve through the reader's theme instead (SQ-1154).",
        shot.id,
        same.iter().map(|s| format!("        {s}")).collect::<Vec<_>>().join("\n")
    ))
}

/// A machine's name from `zvm::interpreter`'s own table, so no tile can be
/// captioned with a machine it was not booted as.
pub fn machine_name(n: u8) -> String {
    zvm::interpreter::machine(n).map_or_else(|| format!("interpreter {n}"), |m| m.name.to_string())
}

/// `Amiga · 4` — what one tile's badge says.
///
/// An unlabelled grid of similar terminals teaches nothing, and the NUMBER is
/// half the label rather than decoration: it is what a reader types to get that
/// tile back (`lanthorn --interpreter 4 --colour machine story.z3`), which makes
/// the frame a thing you can reproduce instead of a thing you can look at.
///
/// Terse because it rides ON the tile now rather than sitting in a strip above it
/// (SQ-1165): `interpreter 4` spelled out was 22 characters looking for a gap in
/// somebody's prose, `--interpreter` is on the frame's own footer already, and a
/// badge only has to be unambiguous rather than complete.
pub fn tile_badge_text(n: u8) -> String {
    format!("{} \u{b7} {n}", machine_name(n))
}

/// Where a tile's badge can sit without covering anything the frame is about.
///
/// **The badge must not cover the evidence, and this is that check rather than a
/// promise about it.** What a composite is FOR is the page colour, the ink, the
/// reverse-video status band and the caret — the band is the pane's first row and
/// the caret sits at the prompt — so the free ground is below the prose. But the
/// six tiles do not fill to the same height (a machine whose interpreter wraps a
/// line differently ends a row lower), and Deadline's opening is not the last
/// story this manifest will ever point a composite at. So the spot is FOUND, per
/// tile, off that tile's own resolved screen: the lowest two-row band of the pane
/// with a clear run wide enough, scanning up.
///
/// Returns the top-left in PIXELS, in the space [`super::raster`] draws the frame
/// in, and `None` when the pane has no clear run at all — which the caller reports
/// rather than papering over, because a badge dropped on the prose is the one
/// outcome worse than no badge.
pub fn badge_anchor(shot: &Shot, res: &super::oracle::Resolved, cells_wide: u16) -> Option<(u32, u32)> {
    let (cols, rows) = shot.pane_content_cells()?;
    let last_row = u16::try_from(rows).ok()?.min(res.rows.saturating_sub(1));
    let last_col = u16::try_from(cols).ok()?.min(res.cols.saturating_sub(1));
    let clear = |row: u16, col: u16| {
        let c = res.cell(row, col);
        c.image_id.is_none() && matches!(c.ch, ' ' | '\0')
    };
    // Right-aligned inside the pane: the badge ends one cell short of the border,
    // and the run CHECKED reaches one cell further left than it, so there is clear
    // ground on both sides of it rather than a badge butted against a word.
    let left = last_col.checked_sub(cells_wide)?.max(1);
    let checked = left.saturating_sub(1)..=last_col;
    // TWO rows, not one: the badge is taller than a cell, so a single clear row
    // would put its border through the descenders of the line above.
    for row in (2..=last_row).rev() {
        if checked.clone().all(|c| clear(row, c)) && checked.clone().all(|c| clear(row - 1, c)) {
            let (cw, ch) = shot.cell_px();
            return Some((u32::from(left) * u32::from(cw), u32::from(row - 1) * u32::from(ch)));
        }
    }
    None
}

/// Stamp a badge onto a tile, in place.
///
/// **It has to read as ANNOTATION and not as something lanthorn drew**, which is
/// the whole difficulty: every tile is a picture of a terminal app with its own
/// title bar, its own help line and its own borders, and a tag dropped carelessly
/// into that becomes a claim about what the app renders — the worst possible
/// misreading of a frame whose subject is exactly that. Three things separate it,
/// and all three are deliberate:
///
///   * drawn in the harness's own BITMAP master at 8px, never the frame's
///     typeface — the same rule [`label`] follows, and for the same reason;
///   * on the label strip's own near-black ground under a bright hairline border,
///     a combination lanthorn's theme has nowhere;
///   * inset INSIDE the pane rather than straddling a border, so it can never be
///     read as a piece of the app's chrome that happens to have a word in it.
pub fn stamp_badge(frame: &mut RgbaImage, text: &str, at: (u32, u32)) {
    const PAD: u32 = 5;
    let (x, y) = at;
    let w = PAD * 2 + 8 * text.chars().count() as u32;
    let h = PAD * 2 + LABEL_LINE;
    for dy in 0..h {
        for dx in 0..w {
            let (px, py) = (x + dx, y + dy);
            if px >= frame.width() || py >= frame.height() {
                continue;
            }
            let edge = dx == 0 || dy == 0 || dx + 1 == w || dy + 1 == h;
            frame.put_pixel(px, py, if edge { Rgba([122, 126, 140, 255]) } else { Rgba([18, 18, 20, 255]) });
        }
    }
    for (j, ch) in text.chars().enumerate() {
        app::render::bitfont::blit_glyph(
            frame,
            ch,
            x + PAD + j as u32 * 8,
            y + PAD,
            8,
            LABEL_LINE,
            Rgba([214, 216, 224, 255]),
            None,
            None,
        );
    }
}

/// How many CELLS wide the badge for `text` is, so [`badge_anchor`] can ask for a
/// clear run of the right size before [`stamp_badge`] draws into it.
///
/// One function rather than the same arithmetic in two places: the anchor and the
/// stamp must agree about the badge's width or the check is of a box the drawing
/// does not use, which is a guard that passes while the badge covers prose.
pub fn badge_cells(shot: &Shot, text: &str) -> u16 {
    let px = 10 + 8 * text.chars().count() as u32;
    let (cw, _) = shot.cell_px();
    u16::try_from(px.div_ceil(u32::from(cw))).unwrap_or(u16::MAX)
}

/// Lay the captured tiles out in a grid on the label strip's own ground.
///
/// Each tile has already named ITSELF, through [`stamp_badge`], so this is only
/// the arithmetic. There was a caption strip above every tile until the badge
/// arrived: two labels saying the same thing is clutter, and the one drawn on the
/// picture is the one that survives the picture being cropped.
///
/// [`TILE_GUTTER`] carries why there is any ground between them at all, and
/// [`Shot::tile_columns`] picks `columns` from the shape this produces.
pub fn tile(panels: &[RgbaImage], columns: usize) -> RgbaImage {
    let ground = Rgba([18, 18, 20, 255]);
    let cols = columns.max(1);
    let rows = panels.len().div_ceil(cols);
    let cell_w = panels.iter().map(RgbaImage::width).max().unwrap_or(1);
    let cell_h = panels.iter().map(RgbaImage::height).max().unwrap_or(1);
    let w = TILE_GUTTER + (cell_w + TILE_GUTTER) * cols as u32;
    let h = TILE_GUTTER + (cell_h + TILE_GUTTER) * rows as u32;
    let mut out = RgbaImage::from_pixel(w.max(1), h.max(1), ground);
    for (i, frame) in panels.iter().enumerate() {
        let x0 = TILE_GUTTER + (cell_w + TILE_GUTTER) * (i % cols) as u32;
        let y0 = TILE_GUTTER + (cell_h + TILE_GUTTER) * (i / cols) as u32;
        for (x, y, p) in frame.enumerate_pixels() {
            let (px, py) = (x0 + x, y0 + y);
            if px < out.width() && py < out.height() {
                out.put_pixel(px, py, *p);
            }
        }
    }
    out
}

// ── Type ──────────────────────────────────────────────────────────────────────

/// A glyph face for the gallery: the harness's own bitmap master, or a real
/// outline font loaded from disk.
///
/// The outline path exists only here. `raster::render`'s default is untouched
/// and the tests never reach this type, so the geometry oracle goes on looking
/// as synthetic as it should.
pub enum Face {
    /// [`app::render::bitfont`] — Uni-VGA 8x16, the face the v6 pixel composite
    /// itself draws with.
    Bitmap,
    /// A TrueType face rasterised at the size its own metrics put in the cell.
    Outline {
        name: String,
        font: Box<fontdue::Font>,
        px: f32,
        /// The cell this face's own metrics round to at `px`: `round(advance)`
        /// by `round(line height)`. Equal to the shot's cell when the size was
        /// chosen well, and worth printing when it is not.
        natural: (u32, u32),
        /// A second face, tried only for a glyph the primary face declined
        /// (SQ-1229). `▸` (the matrix view's current-room marker) and `⇄` (a
        /// reciprocal-exit marker) are outside Fira Code Nerd Font Mono's
        /// range, and a real terminal supplies them from the OS's own font
        /// fallback — which is why nobody notices this live. `None` when no
        /// candidate in [`FALLBACK_FONT_CANDIDATES`] loads.
        fallback: Option<(Box<fontdue::Font>, f32)>,
        /// Every character neither this face, the fallback, nor the bitmap
        /// master could draw.
        ///
        /// The reason this quest exists is that a missing glyph is SILENT: the
        /// map's arrowheads came out as `.notdef` boxes under Monaco and the run
        /// reported nothing at all. A blank cell is quieter still, so the ones
        /// that get this far are counted and named at the end of the run.
        unresolved: std::cell::RefCell<BTreeSet<char>>,
    },
}

impl Face {
    /// Load a TTF and size it to the cell FROM THE FACE'S OWN METRICS.
    ///
    /// Every glyph in a terminal occupies exactly one cell, so the rasterisation
    /// that belongs here is the one whose natural line box IS the cell: `px =
    /// cell_h / new_line_size(1px)`. That used to be `cell_h * 0.78`, a constant
    /// that happens to be near the truth for some faces and not for others, and
    /// which quietly stretched or shrank the type against the cell it sat in.
    ///
    /// For the default face at the two cells this tool captures at, the answer is
    /// one of the sweet-spot sizes the quest names: 13px in an 8x16 cell, 16px in
    /// a 10x20 one (SQ-0963). A face with no horizontal line metrics at all keeps
    /// the old constant, because something has to be drawn.
    pub fn outline(path: &Path, cell_h: u16) -> Result<Face, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("font {}: {e}", path.display()))?;
        let font = fontdue::Font::from_bytes(bytes.as_slice(), fontdue::FontSettings::default())
            .map_err(|e| format!("font {}: {e}", path.display()))?;
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "font".into());
        let per_px = font.horizontal_line_metrics(1.0).map(|m| m.new_line_size).filter(|v| *v > 0.0);
        let px = size_for_cell(&font, cell_h);
        // The advance is the same for every glyph in a monospace face, so `M`
        // answers for all of them.
        let natural = (
            font.metrics('M', px).advance_width.round().max(1.0) as u32,
            per_px.map_or(u32::from(cell_h), |line| (line * px).round().max(1.0) as u32),
        );
        Ok(Face::Outline {
            name,
            font: Box::new(font),
            px,
            natural,
            fallback: load_fallback_font(cell_h),
            unresolved: Default::default(),
        })
    }

    /// How the label should name this face — which is the whole reason a real
    /// one is allowed at all.
    pub fn describe(&self) -> String {
        match self {
            Face::Bitmap => "Uni-VGA 8x16 (the harness's own bitmap face)".to_string(),
            Face::Outline { name, px, natural, .. } => {
                format!("{name} rasterised at {px:.0}px ({}x{}) by the harness", natural.0, natural.1)
            }
        }
    }

    /// Whether this face's own cell at its chosen size IS the cell it is drawn
    /// into. A complaint when it is not — never fatal, because a reader can still
    /// judge layout from slightly wrong type, but printed, because the whole
    /// reason a size is pinned is that nobody notices a drift by eye.
    pub fn cell_complaint(&self, cell_w: u16, cell_h: u16) -> Option<String> {
        match self {
            Face::Bitmap => None,
            Face::Outline { name, px, natural, .. } => (*natural != (u32::from(cell_w), u32::from(cell_h)))
                .then(|| {
                    format!(
                        "{name} at {px:.0}px has a {}x{} cell, but this shot is captured at {cell_w}x{cell_h} — \
                         the type will not sit square in it (SQ-0963 pins a face whose cell is exactly 1:2)",
                        natural.0, natural.1
                    )
                }),
        }
    }

    /// Characters no face in the chain could draw, in codepoint order.
    pub fn unresolved(&self) -> Vec<char> {
        match self {
            Face::Bitmap => Vec::new(),
            Face::Outline { unresolved, .. } => unresolved.borrow().iter().copied().collect(),
        }
    }

    /// Draw one glyph into a cell.
    pub fn draw(&self, canvas: &mut RgbaImage, ch: char, px: u32, py: u32, cw: u32, chh: u32, fg: Rgba<u8>) {
        match self {
            Face::Bitmap => app::render::bitfont::blit_glyph(canvas, ch, px, py, cw, chh, fg, None, None),
            Face::Outline { font, px: size, fallback, unresolved, .. } => {
                // The half-block and box-drawing glyphs are the picture's
                // STRUCTURE — rules, borders, and every pixel of a half-block
                // frame. A text face either lacks them or draws them with gaps
                // at the cell seams, so they stay with the bitmap master whose
                // cells tile exactly.
                //
                // And then the CAPABILITY question, which is the durable half
                // (SQ-0963). The old rule was a RANGE — U+2500..=U+259F — so the
                // map's arrowheads, which are Arrows and Geometric Shapes, went
                // to fontdue, which drew `.notdef`. Widening the range would fix
                // that one set of glyphs for that one face; asking the face
                // whether it HAS the glyph fixes it for every face anyone passes
                // to `--font`, including the ones nobody has thought of.
                //
                // A glyph the PRIMARY face declines gets one more chance before
                // the bitmap master (SQ-1229): the fallback face, tried only
                // here, never as the primary. `▸`/`⇄` are outside Fira Code Nerd
                // Font Mono's range but inside a face like DejaVu Sans Mono or
                // Menlo's — exactly the OS fallback a real terminal supplies
                // without anyone noticing.
                let resolved = if is_structural(ch) {
                    None
                } else if font.has_glyph(ch) {
                    Some((font.as_ref(), *size))
                } else {
                    fallback.as_ref().and_then(|(f, p)| f.has_glyph(ch).then(|| (f.as_ref(), *p)))
                };
                let Some((draw_font, draw_size)) = resolved else {
                    app::render::bitfont::blit_glyph(canvas, ch, px, py, cw, chh, fg, None, None);
                    // The master is a short hand-authored list, not a font: it
                    // covers font 3, the ZSCII table and the runes, and nothing
                    // says it covers whatever neither face just declined. Record
                    // what fell through all three, so the next silent gap is a
                    // printed line rather than a blank cell somebody eventually
                    // notices.
                    if !is_structural(ch) && !ch.is_whitespace() && !app::render::bitfont::has_glyph(ch) {
                        unresolved.borrow_mut().insert(ch);
                    }
                    return;
                };
                let (m, bitmap) = draw_font.rasterize(ch, draw_size);
                if m.width == 0 || m.height == 0 {
                    return;
                }
                // Baseline at 80% of the cell, glyph centred horizontally: a
                // terminal advances by the cell, not by the glyph's own width.
                let baseline = py as i64 + (i64::from(chh) * 4) / 5;
                let x0 = px as i64 + (i64::from(cw) - m.width as i64) / 2;
                let y0 = baseline - m.height as i64 - i64::from(m.ymin);
                for gy in 0..m.height {
                    let y = y0 + gy as i64;
                    if y < 0 || y >= i64::from(canvas.height()) {
                        continue;
                    }
                    for gx in 0..m.width {
                        let x = x0 + gx as i64;
                        if x < 0 || x >= i64::from(canvas.width()) {
                            continue;
                        }
                        let a = u32::from(bitmap[gy * m.width + gx]);
                        if a == 0 {
                            continue;
                        }
                        let dst = canvas.get_pixel(x as u32, y as u32).0;
                        let mix = |s: u8, d: u8| ((u32::from(s) * a + u32::from(d) * (255 - a)) / 255) as u8;
                        canvas.put_pixel(
                            x as u32,
                            y as u32,
                            Rgba([mix(fg[0], dst[0]), mix(fg[1], dst[1]), mix(fg[2], dst[2]), 255]),
                        );
                    }
                }
            }
        }
    }
}

/// Glyphs that are structure rather than type: half-blocks, shades, and the box
/// drawing range. These must tile with no seam, which only the bitmap master
/// does.
fn is_structural(ch: char) -> bool {
    matches!(ch, '\u{2500}'..='\u{259F}')
}

/// Monospace faces worth trying when `--font` was not given, in order. `~/`
/// means the user's home directory; nothing else is expanded.
///
/// **Fira Code leads, and it is a measurement rather than a taste** (SQ-0963).
/// A half-block sample is `cell_width` wide by `cell_height / 2` tall, so square
/// samples want a cell of exactly 1:2, and a face's cell is `round(advance · px)`
/// by `round(line · px)` — the two round at different rates, so what matters is
/// how often the ROUNDED cell lands on 2.000 rather than what the em ratio says.
/// Measured off the sfnt tables, over 6..24 px/em:
///
/// | face | advance/line (em) | ratio | rounded cells that hit 2.000 |
/// |---|---|---|---|
/// | Fira Code Nerd Font | 0.615 / 1.231 | **2.000** | 10 of 19 — 5x10, 6x12, 7x14, 8x16, 9x18, 10x20, 11x22, 13x26, 14x28, 15x30 |
/// | 0xProto Nerd Font Mono | 0.620 / 1.200 | 1.935 | — |
/// | Source Code Pro Nerd Font Mono | 0.600 / 1.257 | 2.095 | — |
/// | JetBrains Mono Nerd Font Mono | 0.600 / 1.320 | 2.200 | 1 of 19, at 4x8 |
/// | Monaco | 0.600 / 1.333 | 2.222 | — |
/// | Iosevka Term Nerd Font Mono | 0.500 / 1.250 | 2.500 | — |
///
/// Those ten sizes are the historical terminal cells, and [`Shot::cell_px`]
/// captures at two of them. JetBrains Mono — which this list led with, chosen on
/// glyph coverage back when coverage was load-bearing — is 10% off at every size
/// anyone would pick, so its shots sampled the artwork coarser down than across.
///
/// Coverage is no longer the deciding question, because [`Face::draw`] asks the
/// face whether it HAS each glyph and falls back to the bitmap master when it
/// does not. Worth knowing anyway: Fira Code does carry the map's arrowheads
/// (`↑ ↓ ▲ ▼ ◀ ▶`, verified against its `cmap`) and does NOT carry `⊙`/`⊗`, which
/// JetBrains Mono did. Neither does the bitmap master, so a frame containing one
/// is named in the run's output rather than silently losing it. That hole is why
/// the portal badges are no longer those two codepoints: the default in/out pair
/// is `◉`/`◎` from Geometric Shapes, which Fira Code does carry (SQ-0989).
///
/// Deliberately short and platform-obvious. `.ttc` collections are skipped —
/// fontdue reads a single face — so this list is plain `.ttf` only.
pub const FONT_CANDIDATES: &[&str] = &[
    "~/Library/Fonts/FiraCodeNerdFontMono-Regular.ttf",
    "/Library/Fonts/FiraCodeNerdFontMono-Regular.ttf",
    "~/.local/share/fonts/FiraCodeNerdFontMono-Regular.ttf",
    "/usr/share/fonts/truetype/firacode/FiraCodeNerdFontMono-Regular.ttf",
    "/usr/share/fonts/TTF/FiraCodeNerdFontMono-Regular.ttf",
    "/usr/share/fonts/truetype/firacode/FiraCode-Regular.ttf",
    "/System/Library/Fonts/Menlo.ttf",
    "/System/Library/Fonts/Monaco.ttf",
    "/System/Library/Fonts/Supplemental/Andale Mono.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
    "/usr/share/fonts/TTF/DejaVuSansMono.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf",
];

/// A candidate's path with a leading `~/` resolved against `$HOME`.
///
/// Nerd Fonts install per-user on both macOS and Linux — `~/Library/Fonts` and
/// `~/.local/share/fonts` — so a list of absolute paths could not name the face
/// this tool is supposed to lead with.
pub fn candidate_path(cand: &str) -> Option<PathBuf> {
    match cand.strip_prefix("~/") {
        Some(rest) => std::env::var_os("HOME").map(|h| PathBuf::from(h).join(rest)),
        None => Some(PathBuf::from(cand)),
    }
}

/// The first candidate that loads, or the bitmap face.
pub fn pick_face(explicit: Option<&Path>, cell_h: u16) -> Result<Face, String> {
    if let Some(p) = explicit {
        return Face::outline(p, cell_h);
    }
    for cand in FONT_CANDIDATES {
        let Some(p) = candidate_path(cand) else { continue };
        if p.is_file() {
            if let Ok(f) = Face::outline(&p, cell_h) {
                return Ok(f);
            }
        }
    }
    Ok(Face::Bitmap)
}

/// The size, in px, at which a face's own line metrics fill `cell_h` — shared
/// by [`Face::outline`] and [`load_fallback_font`] so a fallback face lands in
/// the cell the same way the primary one does.
fn size_for_cell(font: &fontdue::Font, cell_h: u16) -> f32 {
    let per_px = font.horizontal_line_metrics(1.0).map(|m| m.new_line_size).filter(|v| *v > 0.0);
    match per_px {
        Some(line) => (f32::from(cell_h) / line).round().max(1.0),
        None => f32::from(cell_h) * 0.78,
    }
}

/// Faces tried only for a glyph [`FONT_CANDIDATES`]' pick declined — never as
/// the primary face itself, which is a measurement (see that list's own docs),
/// not a preference (SQ-1229).
///
/// The gap this closes: `▸` (the matrix view's current-room marker,
/// `render/matrix.rs`) and `⇄` (a reciprocal-exit marker, `render/room_info.rs`)
/// are outside Fira Code Nerd Font Mono's range, so the harness drew both
/// BLANK — a `maze-grid` still and the automap losing its current-room marker
/// went uncaught because a real terminal's OS font fallback supplies them
/// live, and only this rasteriser has no such fallback of its own.
///
/// DejaVu Sans Mono is the traditional broad-coverage face on Linux, already
/// installed at these paths on most distributions. Menlo is every Mac's own
/// system monospace since 10.6 and needs no install; unlike [`FONT_CANDIDATES`]
/// this list may name it as its `.ttc`, because `fontdue::FontSettings`'s
/// `collection_index` defaults to `0` — the first face in the collection loads
/// with no extra plumbing, whereas the primary-face list has no way to prefer
/// a later face in one and so leaves collections alone entirely.
pub const FALLBACK_FONT_CANDIDATES: &[&str] = &[
    "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
    "/usr/share/fonts/TTF/DejaVuSansMono.ttf",
    "/usr/share/fonts/dejavu/DejaVuSansMono.ttf",
    "/System/Library/Fonts/Menlo.ttc",
];

/// The first [`FALLBACK_FONT_CANDIDATES`] entry that loads, sized to `cell_h`
/// the same way the primary face is — or `None`, which leaves [`Face::draw`]
/// exactly as it behaved before this fallback existed.
fn load_fallback_font(cell_h: u16) -> Option<(Box<fontdue::Font>, f32)> {
    for cand in FALLBACK_FONT_CANDIDATES {
        let Some(p) = candidate_path(cand) else { continue };
        if !p.is_file() {
            continue;
        }
        let Ok(bytes) = std::fs::read(&p) else { continue };
        let Ok(font) = fontdue::Font::from_bytes(bytes.as_slice(), fontdue::FontSettings::default()) else {
            continue;
        };
        let px = size_for_cell(&font, cell_h);
        return Some((Box::new(font), px));
    }
    None
}

// ── The label ─────────────────────────────────────────────────────────────────

/// Height of one label line, in pixels.
const LABEL_LINE: u32 = 16;

/// The ground left between a composite's tiles, and around them (SQ-1165).
///
/// Not cosmetic. The Macintosh tile is white to its edge and the Apple tile is
/// black to its edge, and two pages meeting with no ground between them read as
/// one surface with a seam in it.
///
/// Shared with [`Shot::tile_columns`], which chooses a column count by the shape
/// of the picture this file actually writes — a grid picked against different
/// arithmetic than the one drawing it is exactly the hand-maintained invariant
/// across call sites that CLAUDE.md's refactoring policy is about.
const TILE_GUTTER: u32 = 14;

/// Append a footer to `frame` saying, in the picture itself, what the picture is.
///
/// WHY IT IS BURNT IN AND NOT A CAPTION. An image gets separated from its page
/// the first time somebody drags it into a chat window, and the claim that
/// survives that trip is the one inside the pixels. The label is always drawn
/// with the BITMAP face, whatever the frame above it used: a footer that shares
/// the frame's typeface reads as part of the render, and this one has to read as
/// the harness talking about the render.
pub fn label(frame: &RgbaImage, lines: &[String]) -> RgbaImage {
    const PAD: u32 = 4;
    let w = frame.width().max(1);
    // WRAPPED, never clipped. A provenance line that runs off the right edge
    // loses the seed or the release, and the label's whole job is that those
    // travel with the picture.
    let cols = ((w.saturating_sub(PAD * 2)) / 8).max(8) as usize;
    let rows: Vec<(usize, String)> = lines
        .iter()
        .enumerate()
        .flat_map(|(i, l)| wrap(l, cols).into_iter().map(move |r| (i, r)))
        .collect();

    let strip = LABEL_LINE * rows.len() as u32 + PAD * 2;
    let mut out = RgbaImage::from_pixel(w, frame.height() + strip, Rgba([18, 18, 20, 255]));
    for (x, y, p) in frame.enumerate_pixels() {
        out.put_pixel(x, y, *p);
    }
    // A hairline between the frame and the provenance under it, so the two are
    // never read as one surface. It was red while the first label line was a
    // warning; with the warning gone, red would be a signal about nothing.
    for x in 0..w {
        out.put_pixel(x, frame.height(), Rgba([70, 72, 78, 255]));
    }
    for (n, (_source, text)) in rows.iter().enumerate() {
        let fg = Rgba([150, 152, 158, 255]);
        let y = frame.height() + PAD + LABEL_LINE * n as u32;
        for (j, ch) in text.chars().enumerate() {
            app::render::bitfont::blit_glyph(&mut out, ch, PAD + j as u32 * 8, y, 8, LABEL_LINE, fg, None, None);
        }
    }
    out
}

/// Break `text` into runs of at most `cols` characters, at spaces where there is
/// one and mid-word where there is not.
fn wrap(text: &str, cols: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in text.split(' ') {
        let need = if line.is_empty() { word.chars().count() } else { line.chars().count() + 1 + word.chars().count() };
        if need > cols && !line.is_empty() {
            out.push(std::mem::take(&mut line));
        }
        if word.chars().count() > cols {
            // One unbreakable token longer than the strip: cut it rather than
            // let it run off the edge.
            for chunk in word.chars().collect::<Vec<_>>().chunks(cols) {
                if !line.is_empty() {
                    out.push(std::mem::take(&mut line));
                }
                out.push(chunk.iter().collect());
            }
            continue;
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        out.push(line);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

/// The provenance line every gallery frame carries.
///
/// It used to be led by a red "RENDER, NOT A SCREENSHOT" disclaimer, from back
/// when the harness drew the type with its own bitmap master and a frame really
/// did not look like a terminal. It draws with a real terminal face now, so the
/// warning said less than the line below it already does — `face` names the
/// typeface, its size and who rasterised it, which is the honest version of the
/// same fact and is where it stays.
pub fn label_lines(t: &Taken) -> Vec<String> {
    // A COMPOSITE'S TILE SIZE IS NOT THE PICTURE'S SIZE, and the label has to say
    // so: `64x20 cells` under a 3200px frame would read as a claim about the whole
    // image. The machines travel with it too, spelled as the flags that get the
    // tile back, so the frame stays reproducible after it has been dragged out of
    // whatever page it was on (SQ-1165).
    let composite = if t.machines.is_empty() {
        String::new()
    } else {
        format!(
            " | {} tiles, one per machine (--interpreter {} --colour machine), each",
            t.machines.len(),
            t.machines.iter().map(u8::to_string).collect::<Vec<_>>().join("/"),
        )
    };
    vec![
        format!(
            "{} | {} | {} |{composite} {}x{} cells at {}x{}px | {} | {} keypress(es) | seed {} | {}{} | lanthorn {}",
            t.id,
            t.provenance.describe(),
            t.face,
            t.cols,
            t.rows,
            t.cell_w,
            t.cell_h,
            t.backend.as_str(),
            t.turns,
            t.seed,
            t.png.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            // The magnification travels with the picture for the same reason the
            // release does: it is the difference between a frame whose art is
            // pixel-exact and one whose every edge was interpolated, and it is
            // not recoverable by looking (SQ-0963).
            t.scale_note().map(|s| format!(" | {s}")).unwrap_or_default(),
            buildinfo::LONG,
        ),
    ]
}

// ── The contact sheet ─────────────────────────────────────────────────────────

/// A plain HTML index over the frames, so the set can be reviewed in one place
/// before any of it reaches a page.
///
/// Not the website. This is a proof sheet: it exists so whoever regenerates the
/// gallery can see all of it at once and notice the frame that came out wrong.
pub fn contact_sheet(taken: &[Taken], failed: &[String]) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    let _ = writeln!(s, "<!doctype html><meta charset=\"utf-8\"><title>lanthorn gallery (proof sheet)</title>");
    let _ = writeln!(
        s,
        "<style>body{{background:#121214;color:#d8d8dc;font:14px/1.5 system-ui,sans-serif;margin:2rem}}\
         img{{max-width:100%;display:block;border:1px solid #333}}\
         figure{{margin:0 0 3rem}}figcaption{{margin-top:.5rem;color:#a0a0a8}}\
         .warn{{background:#3a1414;border:1px solid #7a2a2a;padding:1rem;margin-bottom:2rem}}\
         code{{color:#c8b48c}}</style>"
    );
    let _ = writeln!(
        s,
        "<div class=\"warn\"><strong>These are renders, not screenshots.</strong> Every frame below was \
         resolved out of the escape bytes the real lanthorn binary wrote to a pty. That makes them honest \
         about layout, art placement and colour, and about nothing else — the type is drawn by the harness, \
         not by anyone's terminal. Hero and marketing shots want a real terminal session.</div>"
    );
    let _ = writeln!(s, "<h1>lanthorn gallery</h1><p>lanthorn <code>{}</code>, {} frame(s).</p>", buildinfo::LONG, taken.len());
    if !failed.is_empty() {
        let _ = writeln!(s, "<div class=\"warn\"><strong>{} shot(s) did not produce a frame:</strong><ul>", failed.len());
        for f in failed {
            let _ = writeln!(s, "<li>{}</li>", escape(f));
        }
        let _ = writeln!(s, "</ul></div>");
    }
    for t in taken {
        let name = t.png.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let _ = writeln!(s, "<figure><img src=\"{}\" alt=\"{}\">", escape(&name), escape(&t.id));
        let _ = writeln!(
            s,
            "<figcaption><code>{}</code> — {} — {} —{} {}x{} cells, {} — {} keypress(es), seed {}{}{}</figcaption></figure>",
            escape(&t.id),
            escape(&t.provenance.describe()),
            escape(&t.face),
            if t.machines.is_empty() {
                String::new()
            } else {
                format!(
                    " {} tiles ({}), each",
                    t.machines.len(),
                    t.machines.iter().map(|&n| machine_name(n)).collect::<Vec<_>>().join(", ")
                )
            },
            t.cols,
            t.rows,
            t.backend.as_str(),
            t.turns,
            t.seed,
            t.scale_note().map(|n| format!(" — {}", escape(&n))).unwrap_or_default(),
            if t.unresolved_glyphs.is_empty() {
                String::new()
            } else {
                format!(
                    " — <strong>no glyph anywhere for {}</strong>",
                    escape(&t.unresolved_glyphs.iter().collect::<String>())
                )
            }
        );
    }
    s
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The regeneration record: what was captured, from what, at what size, with
/// what seed. The PNGs are output; THIS is the thing that says how to get them
/// back, and it sits beside them so a frame found on disk months later can be
/// traced to a build.
pub fn recipe_json(taken: &[Taken], manifest: &Path) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    let _ = writeln!(s, "{{");
    let _ = writeln!(s, "  \"lanthorn\": {},", json_str(buildinfo::LONG));
    let _ = writeln!(s, "  \"manifest\": {},", json_str(&manifest.display().to_string()));
    let _ = writeln!(s, "  \"kind\": \"render — resolved from the escape stream the real binary emitted; not a screenshot\",");
    let _ = writeln!(s, "  \"shots\": [");
    for (i, t) in taken.iter().enumerate() {
        let _ = writeln!(s, "    {{");
        let _ = writeln!(s, "      \"id\": {},", json_str(&t.id));
        let _ = writeln!(s, "      \"png\": {},", json_str(&t.png.display().to_string()));
        // A library shot has no story to describe, and says so in JSON the same
        // way it says so under the frame: nulls, not zeroes. `0` is a release
        // number a story could plausibly carry; `null` is not.
        match &t.provenance {
            Provenance::Story(p) => {
                let _ = writeln!(s, "      \"version\": {},", p.version);
                let _ = writeln!(s, "      \"release\": {},", p.release);
                let _ = writeln!(s, "      \"serial\": {},", json_str(&p.serial));
                let _ = writeln!(s, "      \"medium\": {},", json_str(&p.medium));
                let _ = writeln!(s, "      \"library\": null,");
            }
            Provenance::Library { id, from, members } => {
                let _ = writeln!(s, "      \"version\": null,");
                let _ = writeln!(s, "      \"release\": null,");
                let _ = writeln!(s, "      \"serial\": null,");
                let _ = writeln!(s, "      \"medium\": null,");
                let _ = writeln!(
                    s,
                    "      \"library\": {{\"id\": {}, \"from\": {}, \"members\": {members}}},",
                    json_str(id),
                    json_str(from)
                );
            }
        }
        // The tiles, so a composite found on disk months later can be traced to
        // the six launches behind it and not just to one command line.
        let _ = writeln!(
            s,
            "      \"machines\": [{}],",
            t.machines.iter().map(u8::to_string).collect::<Vec<_>>().join(", ")
        );
        let _ = writeln!(s, "      \"cols\": {}, \"rows\": {},", t.cols, t.rows);
        let _ = writeln!(s, "      \"cell_px\": [{}, {}],", t.cell_w, t.cell_h);
        let _ = writeln!(s, "      \"backend\": {},", json_str(t.backend.as_str()));
        let _ = writeln!(s, "      \"turns\": {},", t.turns);
        let _ = writeln!(s, "      \"seed\": {},", t.seed);
        let _ = writeln!(s, "      \"face\": {},", json_str(&t.face));
        let _ = writeln!(s, "      \"verdict\": {},", json_str(&t.verdict));
        let _ = writeln!(s, "      \"attempts\": {},", t.attempts);
        let _ = writeln!(s, "      \"captured_bytes\": {},", t.captured_bytes);
        match t.native {
            Some((w, h)) => {
                let _ = writeln!(s, "      \"native_px\": [{w}, {h}],");
            }
            None => {
                let _ = writeln!(s, "      \"native_px\": null,");
            }
        }
        match t.magnification {
            Some(m) => {
                let _ = writeln!(s, "      \"magnification\": {m:.6},");
            }
            None => {
                let _ = writeln!(s, "      \"magnification\": null,");
            }
        }
        let _ = writeln!(
            s,
            "      \"unresolved_glyphs\": {},",
            json_str(&t.unresolved_glyphs.iter().collect::<String>())
        );
        let _ = writeln!(s, "      \"png_px\": [{}, {}]", t.width, t.height);
        let _ = writeln!(s, "    }}{}", if i + 1 == taken.len() { "" } else { "," });
    }
    let _ = writeln!(s, "  ]");
    let _ = writeln!(s, "}}");
    s
}

fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two glyphs SQ-1229 found drawn blank: `▸` (the matrix view's
    /// current-room marker, `render/matrix.rs`, and the picker's selection
    /// marker) and `⇄` (a reciprocal-exit marker, `render/room_info.rs`).
    /// Neither Fira Code Nerd Font Mono nor the bitmap master carries them —
    /// `Face::draw` used to fall straight from the primary face to the master
    /// and stop there, so both landed on a blank cell with no complaint beyond
    /// the run's own `NO GLYPH ANYWHERE FOR:` line.
    ///
    /// Skips vacuously when this machine has none of
    /// [`FALLBACK_FONT_CANDIDATES`] installed, the same fixture-absence
    /// pattern every other case here that touches a real file uses — falsify
    /// by temporarily making [`load_fallback_font`] always return `None`
    /// (which is exactly this test's own regression state before SQ-1229):
    /// this case then fails on the `unresolved` assertion below instead of
    /// silently passing.
    #[test]
    fn the_fallback_face_carries_the_automap_markers_no_primary_face_has() {
        let Some(primary) = FONT_CANDIDATES.iter().find_map(|c| {
            let p = candidate_path(c)?;
            p.is_file().then_some(p)
        }) else {
            return; // no primary face on this machine — nothing to rasterise with
        };
        let Some(_fallback) = FALLBACK_FONT_CANDIDATES.iter().find_map(|c| {
            let p = candidate_path(c)?;
            p.is_file().then_some(p)
        }) else {
            return; // no fallback face on this machine — nothing to test
        };

        let face = Face::outline(&primary, 32).expect("a candidate that `is_file()` must parse");
        let mut canvas = RgbaImage::new(16, 32);
        for ch in ['▸', '⇄'] {
            face.draw(&mut canvas, ch, 0, 0, 16, 32, Rgba([255, 255, 255, 255]));
        }

        assert!(
            face.unresolved().is_empty(),
            "U+25B8/U+21C4 must resolve through the fallback face, not fall through to \
             `unresolved` (which is a blank cell): {:?}",
            face.unresolved()
        );
        let painted = canvas.pixels().filter(|p| p.0[3] > 0).count();
        assert!(painted > 0, "drawing ▸ and ⇄ must paint at least one pixel — a blank canvas is the bug SQ-1229 reports");
    }
}
