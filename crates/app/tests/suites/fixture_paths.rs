//! Shared fixture-path resolution (SQ-1015).
//!
//! 125 suites under `tests/suites/` used to each define their own private
//! `stories_dir()` pointing solely at the gitignored, commercial `stories/`
//! directory, so every one of them skipped vacuously on CI. A survey found that
//! a large minority of those suites depend only on fixtures the IF Archive
//! distributes freely — `advent`, `scopa`, `sunburst`, the Mysterious
//! Adventures, `anchor.z8`, `photopia`, and several modern Glulx works — and
//! pointed them at `tests/fixtures/stories/`.
//!
//! That directory is **fetched, not committed** (`scripts/fixtures.manifest`,
//! `scripts/fetch-fixtures.sh`). The IF Archive's Terms of Use presume material
//! with no attached licence is licensed for personal use only, so downloading
//! one is what the Archive is for and republishing it inside this repository is
//! not. CI runs the fetch before `cargo test`; the manifest is the whole of what
//! the repository carries.
//!
//! [`fixture_path`] is the one place that duplication now goes through: it takes
//! the local `stories/` copy when there is one and the fetched copy otherwise, so
//! a developer's run is unchanged and CI — which has no `stories/` — still reaches
//! every fetched fixture. A suite that names a fixture on neither list behaves
//! exactly as it did before: a superset, never a narrowing.
//!
//! **And a skip is no longer ambiguous.** Before the fetch existed, "the fixture
//! is absent" meant one thing on CI: there is no `stories/` there. Now it could
//! also mean the fetch quietly failed, which would turn a broken step into a
//! green run full of silent skips — the exact failure this whole quest is about.
//! `LANTHORN_FIXTURES_REQUIRED=1`, which CI sets after the fetch step and nothing
//! else sets, makes [`fixture_path`] PANIC rather than answer with a path that is
//! not there, for the names the manifest promises and those only.
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The fetch manifest, compiled in so the required-fixture list cannot drift from
/// what the fetch script actually populates. Parsed lazily; see the file's own
/// header for the format.
const MANIFEST: &str = include_str!("../../../../scripts/fixtures.manifest");

/// The destination names `scripts/fixtures.manifest` promises — column 3 of every
/// record line.
fn manifest_names() -> &'static [&'static str] {
    static NAMES: OnceLock<Vec<&'static str>> = OnceLock::new();
    NAMES
        .get_or_init(|| {
            MANIFEST
                .lines()
                .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
                .filter_map(|l| l.split('\t').nth(2))
                .collect()
        })
        .as_slice()
}

/// Resolve `name` against the gitignored local `stories/` first, then the fetched
/// fixtures directory. Returns a path either way (possibly non-existent) so
/// callers keep their existing `std::fs::read(..).ok()?` "skip if absent" pattern
/// unchanged — except under `LANTHORN_FIXTURES_REQUIRED`, where a name the
/// manifest promises and neither directory holds is a panic instead.
pub fn fixture_path(name: &str) -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));

    // LOCAL FIRST, tracked as the fallback — because a story is not the only file
    // a story needs (SQ-1048).
    //
    // The tracked directory holds bare story files; their companion media stay in
    // `stories/`, which is where a release's own graphics, sound and disk resources
    // live. Preferring the tracked copy therefore hands back a story SEPARATED from
    // its resources: `mysterious01.z6` moved here while `Mysterious01.blb` did not,
    // so the title lost its pictures, stopped taking the hybrid ring, and rendered
    // as a full-frame image — which is exactly what `fmvpoker_is_the_only_title_this_moves`
    // exists to catch.
    //
    // Taking the local copy when there is one means a developer's run is bit-for-bit
    // what it was before any fixture moved, and CI — which has no `stories/` at all —
    // still reaches every tracked fixture. That is the "superset, never a narrowing"
    // this module promises; tracked-first quietly broke it for any fixture with a
    // sibling.
    let local = manifest.join("../../stories").join(name);
    if local.is_file() {
        return local;
    }
    let tracked = manifest.join("tests/fixtures/stories").join(name);
    if tracked.is_file() {
        return tracked;
    }

    // Neither has it. If the manifest promised it and the run demanded the
    // manifest be honoured, that is a broken fetch, not a missing shelf — and a
    // vacuous skip would report it as a pass.
    if std::env::var_os("LANTHORN_FIXTURES_REQUIRED").is_some() && manifest_names().contains(&name) {
        panic!(
            "LANTHORN_FIXTURES_REQUIRED is set and `{name}` is in \
             scripts/fixtures.manifest, but it is at neither {} nor {}. \
             The fetch step did not do its job — run `scripts/fetch-fixtures.sh` \
             (or `--verify-only` to see which files are absent). Skipping here \
             would report a broken fetch as a green run.",
            local.display(),
            tracked.display(),
        );
    }

    // Answer in `stories/`, which is where this always pointed and
    // what several callers actually want. `picture_override` asks for
    // `fixture_path("anything.z6")`, a name deliberately on no disk, purely to name
    // the DIRECTORY its real fixtures (`zork0.pic`, `zork0.eg1`) sit beside. Falling
    // back to the tracked directory instead moves that parent and the sidecars vanish.
    local
}
