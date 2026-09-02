# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

lanthorn is a playable interactive-fiction interpreter for the terminal with live automapping. One TUI app (`lanthorn <story-file>`) drives three engines: Z-machine (v3–v8 including graphical v6), Glulx, and Scott Adams. Cross-platform: macOS, Linux, Windows.

## Commands

```sh
cargo build --workspace                 # build everything
cargo run -p app -- stories/foo.z5      # run the TUI (binary name: lanthorn)
cargo nextest run -p app v6_arthur_status            # one integration-test suite
cargo nextest run -p app v6_arthur_status::some_case # one test within it
cargo test -p app --test v6_arthur_advent            # one whole group binary
```

The app's integration suites live in `crates/app/tests/suites/`, which cargo does
**not** auto-build; each is pulled in as a module by one of the ~14 group binaries
at `crates/app/tests/*.rs` (`#[path = "suites/v6_arthur_status.rs"] mod …`). One
link per group instead of one per suite: a `touch crates/blorb/src/lib.rs`
rebuild of `--tests -p app` went from 11.2s to 4.0s, and app's share of `target/`
from 4.3 GiB to 2.8 GiB (SQ-0786). Adding a suite means adding the file under
`suites/` **and** a `mod` line in the group that should carry it — a suite no
group names is never built. Reaching one suite costs nothing extra, because the
module path still carries the old filename: filter by **name** under nextest, as
above, rather than by `--test`.

**Which tests to run when.** COMPILATION dominates turnaround here, not the tests.
Measured at 12 cores: a warm targeted suite is **5.8s** and the full gate **170s**,
but the REBUILD after touching `app` is **151s** and after `zvm` **277s** — and a
filtered run pays that too, because `-p app` links all fourteen group binaries
whatever the filter selects. So selection roughly halves the loop (321s → 157s) and
cannot do better than the build. The linker is already Apple's fast `ld-1267` and the
volume is an SSD; neither is worth chasing.

**Parallelism is capped at 6 on purpose, and the numbers above predate the cap.**
`.cargo/config.toml` sets `jobs = 6` and `.config/nextest.toml` sets
`test-threads = 6`, against a machine with 12 logical cores (8 performance + 4
efficiency). Uncapped, cargo takes every core for the two-to-five minutes a rebuild
runs and the machine is unusable for the whole of it — and clippy is a second full
build with its own fingerprints. Six leaves the four efficiency cores AND two
performance cores for everything else. That trade is deliberate: expect the timings
above to be somewhat worse, and do not "fix" it by raising the cap.

**It was 8 first, and 8 was still too many.** The four efficiency cores were left
alone and the machine was nevertheless sluggish through a rebuild, so the cap came
down to 6 on measured comfort rather than on the core count (2026-08-27). If you
find yourself reasoning from "12 cores, 8 of them fast" to a higher number, that is
the argument that has already been tried.

CI is exempt — both workflows export `CARGO_BUILD_JOBS` from the runner's own core
count before installing Rust, because a 3-4 core runner running six or eight rustc
processes is slower AND puts a crate the size of `app` near the memory ceiling.

- **While iterating**, run the suites that cover what you touched — by NAME under
  nextest (`cargo nextest run -p app v6_arthur_status`), never by `--test`, since the
  module path still carries the old filename.
- **Before you PUSH**, run `cargo check --all-targets` on the tree you are about to
  push — **not** the full gate. CI is the backstop (see below), and a merged-tree
  `cargo check` is what catches the one thing CI would catch too late and nothing
  else local can see: a SEMANTIC merge conflict between parallel lanes, textually
  clean and non-compiling. It costs minutes (see Disk hygiene for the measured
  numbers) where the gate costs many more.
- **Never put the full gate in a parallel lane's brief.** A three-lane wave that
  gates each lane AND the combination pays four full builds — plus four clippy
  builds, which share no fingerprints with them — for one merge. Lanes run
  `cargo check --all-targets` and the suites covering what they touched; the
  integrator runs one `cargo check` on the merged tree. Measured 2026-08-30: this
  was the single largest source of waste in a wave, and it came from a shared brief
  contradicting the line directly above this one.
- **Keep ONE integration worktree warm across waves.** Creating a fresh worktree per
  merge throws away a whole `target/` and pays a cold build to compile code that was
  already compiled in the lanes.
- **A change that compiles nothing needs no gate.** README prose, a doc under
  `docs/`, a committed PNG, a `gallery.toml` key spec — none of it is Rust, and
  the only question worth asking is whether the one suite that READS it still
  passes (`cargo nextest run -p app gallery_manifest` is 23 cases in 0.13s warm).
  The full gate exists for the two things that have ever justified it — a semantic
  merge conflict between parallel lanes, and the shared-process palette races —
  and a line of README prose cannot reach either. Running it anyway costs minutes
  per iteration and buys nothing, which is easy to do a dozen times in a
  documentation pass without noticing.
- **VERIFY BEFORE YOU COMMIT, not after** — `crates/buildinfo/build.rs` shells out
  to git and declares `cargo:rerun-if-changed=<git-dir>/HEAD`, so **every commit
  invalidates `buildinfo`**, and `app` depends on it: the next build relinks all
  fourteen of `app`'s test group binaries whatever you changed. Going from a dirty
  tree to a clean one does it too, because the `-dirty` suffix is part of the same
  string. That is the price of a version that carries its own short hash (which is
  what makes a gallery frame and a bug report self-identifying, so it is worth
  paying) — but pay it once, by running what you need BEFORE the commit rather
  than triggering a relink with the commit and then testing.
- **GitHub Actions runs `cargo test` on every push** and is the real backstop — and
  the only thing that can see a shared-process race, which per-test-process nextest
  structurally cannot (see the palette section below).

Where to look, by what you changed. Prefer more than the obvious one; these are the
floor, not the ceiling:

| changed | run at least |
|---|---|
| `crates/zvm/**` | `-p zvm`, plus the presses you touched (`v6_arthur_advent`, `v6_journey`, `v6_shogun`, `v6_zork0`) |
| `crates/app/src/render/screen.rs` | `v6_render`, `v6_windows`, `zmachine_screen`, `zork_classic` |
| `crates/app/src/render/v6_layout.rs` | `v6_render`, `v6_arthur_advent`, `v6_journey`, `v6_scopa` |
| `crates/app/src/render/transcript.rs` | `zork_classic`, `zmachine_screen`, `-p app --lib` |
| `crates/app/src/native_font.rs`, `crates/blorb/**` | `-p blorb`, `engines`, `v6_zork0` |
| `crates/mapper/**` | `-p mapper`, `mapper_ui` |
| anything touching the PALETTE | the gate **and** `cargo test --workspace` |
| a test that WRITES TO DISK | the gate **and** `cargo test --workspace`, twice |

**Two things have justified the full gate**: a SEMANTIC merge conflict between two
parallel lanes that was textually clean (one lane calling a signature the other had
replaced), and the shared-process palette races of SQ-0904. Note what each actually
needs — the first is a COMPILE failure, so `cargo check --all-targets` on the merged
tree catches it in under a minute; the second is invisible to nextest by
construction and only `cargo test` sees it, which CI runs anyway. Neither is an
argument for running the full test gate locally before a push.

**So the full gate below is CI's job, not the inner loop's** (user decision,
2026-08-30: "no need for full local gate, we catch that with the CI later. we just
need to keep an eye for CI failures"). Run it locally when you actually want it —
a release, a change you distrust, a palette or on-disk change per the table above —
not as a reflex before every push.

**The duty that replaces it: WATCH THE CI RUN.** Trading the local gate for CI makes
an unwatched red run the new failure mode. After a push, check GitHub Actions and
report a failure promptly rather than waiting to be asked.

**Full test gate** (CI runs this; run it locally only when you mean to):

```sh
cargo nextest run --workspace 2>&1 | grep -acE "^error(\[|:)| [1-9][0-9]* failed"
```

This must **print 0**. Note: grep exits 1 when it finds zero matches — that exit code IS the pass, so never chain this with `&&` or treat a nonzero exit as failure. (`-a` because a panicking test can emit a NUL byte, after which grep treats the stream as binary and reports nothing.)

`--workspace`, not a list of `-p` flags. The gate named five crates for months — `blorb`, `zvm`, `gvm`, `scott`, `app` — and every crate outside that list was invisible to it: `mapper`, `audio` and the CLI crates were hundreds of tests the gate could not fail on. A mapper regression could not fail it at all. SQ-0826 found this by removing eleven tests and watching the count drop by one. Naming crates means the gate silently stops covering each new one; `--workspace` cannot go stale, and costs ~30s more. (Deliberately no totals here — a count of what the workspace holds today is an inventory that rots into misinformation, where the line below is a check that recomputes itself every run.)

Cross-check completeness against nextest's own summary rather than by counting lines: `Starting N tests across M binaries` at the top must be followed by `Summary [...] N tests run: N passed`, with the **same N**. A binary that dies mid-run fails the run outright instead of disappearing. That still catches a binary that *died*, not one that was never *enumerated* — a new suite under `crates/app/tests/suites/` that no group binary names is never built, and every count is self-consistent because cargo never saw it. When you add a test file, confirm its name appears in the run (`cargo nextest list | grep <name>`) or just re-run the gate.

`cargo test` still works and is fine for a single binary (above), but it runs test binaries **one at a time**, parallelising only within each. Measured on this workspace at 12 cores: 542s for `cargo test` against 99s for `cargo nextest run`, same 4176 tests — half the binaries carry three tests or fewer while one carries 2343, so global scheduling is worth ~5.5x. Install with `cargo install cargo-nextest --locked`, or the prebuilt binary from <https://get.nexte.st>.

Three consequences of nextest's model worth knowing: it runs **each test in its own process**, so a test that depends on state left behind by another test in the same binary will fail under it (that is a defect, not an incompatibility); and it does not run doctests, which costs us nothing because every crate sets `doctest = false` — if you ever add a real doctest, remove that setting and run `cargo test --doc` alongside.

**And the gate cannot see a shared-process race, because CI runs `cargo test` and the gate does not.** Per-test processes mean no test can observe another's global state, so a race on one is *structurally invisible* to `cargo nextest run` — while `cargo test` gives a binary's tests one process and many threads and hits it. This turned main red four times running (SQ-0904): `zvm::screen::set_palette` is process-global, and twenty-three integration suites each declared their **own** `static PALETTE` mutex, every one documented as "no two cases here may boot at once". True within a suite; meaningless across them, since `tests/suites/*.rs` are modules sharing a group binary's process. Under nextest twenty-three locks are indistinguishable from one, under `cargo test` from zero. They now all take one shared lock, and a source-level case — `palette_lock_discipline` — fails if a suite under `tests/suites/` sets the palette without it, because the *next* such suite is written by someone with no reason to know any of this and the gate cannot catch them (SQ-0905). **When you touch process-global state — the palette is the one we have — verify with `cargo test --workspace` as well as the gate; it is the only command that can answer the question.**

**The palette is not the only process-global thing: so is the filesystem** (SQ-1131). A scratch directory named from `std::process::id()` alone is unique per PROCESS, which under nextest is the same as unique per test and under `cargo test` is unique per *binary* — so every caller of a pid-keyed helper gets the same directory, `fs::write` truncates, and a case's closing `remove_dir_all` deletes what a neighbour is halfway through reading. That is a correct fixture failing its own assertion, somewhere else, intermittently, and it cost eight consecutive red CI runs against a local gate that printed 0 every time (`verb-synonyms-gen`'s `scratch()`, one directory shared by every caller of `wordnet_fixture()`). Inside `app`, take `app::scratch_dir("a-tag")`, which is unique per CALL by construction; in `zvm`/`gvm`/`scott`, which take no dependencies, spell an `AtomicUsize` beside the pid. `scratch_path_discipline` scans every `.rs` file under `crates/` — `src/` `#[cfg(test)]` modules included, which is where most of these live — and asks a helper for the COUNTER, not for a distinguishing name: a scratch path built outside a `#[test]` must have an `AtomicUsize` in the same function (SQ-1163). **A `tag` parameter is not a fix.** It looks like one, because every caller passes a different string, but that is an invariant maintained by hand across call sites in different files, and the moment two spell one the same way it is the bare form again; fifty-one helpers were relying on it, two of them literally `bm-{tag}-{pid}`. The guard still cannot see two `#[test]` bodies that build the same name by different routes — **so a change that adds a test writing to disk wants `cargo test --workspace` too, and wants it twice, because a race that has been fixed passes once by luck as well.**

**And a third process-global class, the nastiest of the three: THREAD-AFFINE OS HANDLES** (SQ-1162). An audio device is not a value you can hold twice — cpal keeps a process-global `static ENUMERATOR` on Windows while initialising COM in a `thread_local` whose `Drop` calls `CoUninitialize()`, so a finished libtest thread can unload MMDevAPI out from under that global pointer. The symptom is not an assertion: the whole binary dies with `0xc0000005 STATUS_ACCESS_VIOLATION` and NO test reports failure, so the printed tail is scheduling rather than causation and naming the culprit needs `--test-threads=1`. On macOS the same shape merely crawls — four cases went from 0.76s serial to **491.54s** in parallel, real CoreAudio streams torn down on threads that never opened them.

**Nextest is structurally blind to it, exactly as with the palette**: one process per test is never two threads. It reddened Windows CI while the local gate was green.

The trap is that the offending call is INVISIBLE at the call site. `Action::ConfigSave` builds an `AudioBackend` whenever `enable_sound` is on and the state holds none — which is every `AppState::default()` — so a *settings* case opens a real device without the word "audio" appearing in its body. The `audio` crate's own rule ("call `disable_output_for_tests()` in any test that constructs a backend") could not be followed by someone who did not know they were constructing one. It is therefore said ONCE, in `AppState::default()` under `#[cfg(test)]`: the lazy construction still runs and is still asserted on, only the device open is skipped, and the shipped binary never compiles the line. **Do not push that rule back out to the call sites** — that is the arrangement that failed.

**The rule has a reader half too: no test may assume a palette it did not write** (SQ-0958). A suite that never sets one still *resolves* colour numbers, through whatever the last suite in its group binary left behind — which is `Standard` under nextest always, and a machine's table under `cargo test` as soon as a sibling boots a press. `v6_shogun_gameplay` asserted §8.3.1 white while `v6_shogun_title_header` booted the same story as an IBM PC, and read `Rgb(173, 173, 173)` instead; main was red on it for exactly as long as the local gate said 0. So every suite that asserts a colour states its palette in one call that also takes the lock — `let _g = app::v6_palette(zvm::screen::Palette::Standard);`, held for the whole case — and assuming the default is as much an assumption as any other. `palette_lock_discipline`'s second case enforces it; a writer is a call and easy to see, a reader is an ABSENCE, so that case matches on booting/rendering **plus** asserting a colour, by literal (`Rgb(`, `Rgba(`) or by painted surface (`RgbaImage`, `paint_surface`, …). The surface half is not theoretical: a suite comparing two grounds names no colour at all and still broke the moment a sibling flipped the table between them.

**And the guard puts `Standard` back when it drops** (SQ-0959), so a case that names a palette leaves the process on the default rather than on the last machine it booted — which is the table nextest's fresh process would have given the next case. It restores the DEFAULT, not the value it displaced, because restore-previous is only meaningful if every writer restores.

**Every writer now does, and there is no other way to write** (SQ-0987). Three locks on one route, and you only ever meet the first one that catches you: the shared lock is **private to `app`**, so a suite cannot take it raw — that is a compile error, not a convention; `app::v6_set_palette` is the only reachable setter and **panics** unless the calling thread holds a guard; and `palette_lock_discipline` fails any file under `tests/suites/` that reaches `zvm::screen::set_palette` directly, which is the one spelling the other two cannot see. So the two ways in are `let _g = app::v6_palette(p);` when the case can name its table at the lock site, and `let _g = app::v6_palette_at_boot();` when it cannot — thirty harnesses resolve an `InterpreterProfile` from a medium deep inside their own `boot()` and set `profile.palette()` there, several rows below where the lock is taken. `v6_palette_at_boot` is exactly `v6_palette(Standard)` with permission to name another table later: it still installs a known palette rather than leaving whatever was there, because "leave whatever was there" is how SQ-0958 happened. **Do not add a "lock now, set later" helper that skips that** — the pairing is the rule. The invariant this buys is that the palette outside the lock is always `Standard`, so the table a case inherits under `cargo test` is the one nextest's fresh process would have given it.

**Clippy gate** — CI's, not the inner loop's, for the same reason as the test gate:
`cargo clippy --workspace --all-targets -- -D warnings` must be clean, and CI runs
exactly that on every push. It costs ~149s the first time after a test build
(separate fingerprints, so it shares NOTHING with it — running the test gate and
then clippy is two complete builds of the same code) and ~0.3s when already warm.
Locally, narrow it to the crate you edited if you run it at all.

**But do NOT reach for the workspace sweep in the inner loop — narrow it to what you touched.** CI runs exactly `cargo clippy --workspace --all-targets -- -D warnings` on every push (`.github/workflows/test.yml`, Linux only, because clippy's result does not vary meaningfully by OS), so the full sweep already has a backstop and running it locally per iteration buys very little for minutes a time.

**Clippy cannot take a list of FILES, and never will.** It is a rustc driver, and Rust's compilation unit is the crate: to lint one module it must parse, macro-expand, name-resolve and type-check the whole crate, because what code in one file means depends on every other file in it. The only granularity on offer is the package (`-p`) and the target within it:

| scope | what it covers | measured here |
|---|---|---|
| `--workspace --all-targets` | everything, incl. all fourteen of `app`'s test group binaries | ~150s+ |
| `-p app --all-targets` | one package, still all its test binaries | most of that |
| **`-p app --lib`** | one package, library target only | **62s** |

So for a change confined to `crates/app/src/`, `cargo clippy -p app --lib` is the local gate and CI is the sweep. Match the `-p` to the crate you edited; add `--all-targets` only when you actually changed something under `tests/`.

And note WHY even the narrow run costs a minute: at 62s wall it burned 2.75s of CPU at 33% utilisation. Almost none of that is lint work — it is rebuilding `app` under clippy's own fingerprints, which share nothing with the test build. Running the test gate and then clippy is two full builds of the same code, and no amount of scoping changes that; only doing it once, at the end, does.

## Hard rules

- **`zvm`, `gvm`, and `scott` take ZERO external dependencies.** All parsing, text codecs, and Quetzal/save handling are hand-rolled. CLI crates and `app` may add deps (crossterm, ratatui, etc.).
- **Stage files explicitly by path.** Never `git add -A` / `git add .` — the working tree routinely carries untracked scratch files and gitignored fixtures that must not be committed. Delete any `scratch_*.rs` test files before committing.
- **No GitHub PRs.** Workflow is: work on main for routine changes (a feature branch + local merge for major work), then `git push origin HEAD:main`.
- **Commit trailers**: a git hook requires a quest trailer on every commit — `Quest: SQ-xxxx` (work in progress), `Completes: SQ-xxxx` (closes it), `Confirm: SQ-xxxx` (done but awaiting user verification), or `Quest: none`. Quests are tracked with the side-quest MCP tools / `side-quest` CLI, not files.
  - **The commit that finishes the work closes the quest.** `Quest:` only advances a quest to `partial`; nothing closes it later on its own. Use `Completes:` when the work is done and a test or an obvious check settles it, and `Confirm:` when only the user's eye can (rendering, interaction feel, audio, a real-game smoke you cannot run). Reach for `Quest:` only when the commit genuinely leaves the quest unfinished.
  - This bites hardest in **parallel worktree lanes**: every lane brief must say which trailer to end on, because a lane that ships its whole feature under `Quest:` parks a finished quest at `partial` and nobody notices until an audit. One such wave left fourteen quests stranded — SQ-0713, 0726, 0734, 0786, 0789, 0790, 0794 and 0798 were all complete, gated and merged, and all still read as outstanding. Before closing out a wave, list the quests it touched and check each one's status is the one you meant.
- **Verify spec constants against authoritative sources** (Z-Machine Standards Document, Glk/Glulx specs), never from memory — unit tests that share the implementation's wrong assumption pass anyway. VM/protocol features need a real-game smoke test.
- **Remove a worktree as soon as its branch is merged.** Each one carries its own `target/` — measured at 4.7–6.8 GB — which is pure garbage the moment the branch lands, and cargo never reclaims it. Five merged worktrees held 27 GB. The check and the removal, from the main checkout:
  ```sh
  git log --oneline main..<branch> | wc -l      # 0 means fully merged
  git worktree remove --force <path> && git branch -D <branch> && git worktree prune
  ```
  Do this in the same breath as the merge, not "later" — the cost is invisible until it is enormous.

## Refactoring policy

**Facts that must be considered TOGETHER should travel together as a value, not
positionally.** When a function takes several parameters that are really one
subject — the machine, the frame, the request — a caller who supplies a subset
gets a *plausible* answer rather than an error, and the resulting defect is
silent, self-consistent, and survives review. Adding the next fact then edits
every call site again, which is when the omissions get made.

The tell is a parameter list where two or more arguments always come from the
same place, or a comment somewhere promising that another file does the same
thing in the same order. **A hand-maintained invariant across files is the
symptom**; the cure is a type.

Measured here, repeatedly, always in the same shape — numbers that are entirely
self-consistent and describe a screen the player never sees:

- SQ-0901: two harnesses omitted `native_std_window`, so 560x384 presses were
  measured at 640x400. A whole quest was fixed and tested against the fabricated
  Arthur frame that produced.
- SQ-1020: `ring_scout` omitted the v6 cell — in the instrument built to catch
  SQ-0901 — so every Macintosh frame it reported was laid out on 8x16.
- SQ-1021: the same omission across twelve Macintosh render harnesses, and a
  pinned window height of 320 that no Macintosh can produce (`320/15 = 21.33`).
- SQ-1022: `reset.rs` omitted the cell in PRODUCTION, so `@restart` re-booted a
  Macintosh game on a different grid than its launch — three lines below a comment
  promising "the same four links `startup.rs` resolves, in the same order".

`app::machine_boot::MachineBoot` (the five per-machine boot facts) and
`render::v6_layout::FrameGeometry` (unit screen, art density, text cell) are the
two that exist. Prefer adding a fact to one of those over adding a parameter.

**Corollary — a guard beats a convention.** Where the wrong spelling cannot be
made unreachable, add a source-level case that fails it, the way
`palette_lock_discipline` and
`render::screen::tests::no_bare_v6_cell_literals_in_native_pixel_arithmetic` do.
The next person to write `py + 16` has no reason to know any of this.

**And do not regex Rust source to perform these conversions.** Multi-line call
sites, method definitions that look like call sites, and arguments spanning lines
all match patterns you did not intend; a regex sweep here has three times produced
code that compiled into something subtly different or mangled a function
signature into a call. Parse by paren balance, verify the shape you expect,
**skip anything unrecognised rather than guessing**, and then check the
conversion against the original (for SQ-1021 that meant diffing each converted
file's `.or(...)` link against `git show HEAD:`, which caught a dropped
named-archive link). Convert the awkward remainder by hand.

## Disk hygiene

Cargo has no garbage collection for `target/`: every hash change writes a new artifact beside the old one and orphans it forever (`-Zgc` is nightly and reclaims the *registry* cache, not build output). Two things dominate, and neither needs a tool:

- **The build directory is `target.noindex/`, not `target/`** (`.cargo/config.toml`
  sets `target-dir`), because Spotlight indexes everything on this volume except a
  directory whose name ends in `.noindex`, and it was indexing every build:
  `corespotlightd` at 200% CPU behind a `cargo check --all-targets` that took
  15–28 minutes during a wave of merges (2026-09-02). CI and the Dockerfile set
  `CARGO_TARGET_DIR=target` so their `target/...` paths still hold. Anything that
  walks the repo tree must skip every directory whose name STARTS with `target`.
- **`target.noindex/debug/incremental`** is a pure cache — delete it freely; the only cost is a slower next build. It reached 48 GB (2,020 sessions) after one day of eleven lanes; deleting it changed nothing about check time, which is the point below.
- **A merged-tree `cargo check --all-targets` is not "under a minute" for this crate any more.** Measured on a quiet machine with a fresh cache (2026-09-02): `-p app --lib` 33s, `-p app --lib --tests` 5m19s, `--all-targets` after touching `app` 11m. The cost is `app`'s library test module — ~3,000 unit tests compiled as one unit with the lib — plus the fourteen group binaries. Only a crate split moves it.
- **Merged worktrees** — see the hard rule above.

For the orphaned artifacts themselves there is `cargo sweep`, but **do not run it routinely here** — build speed beats disk, and an occasional manual `cargo clean` is the preferred trade. Measured on this workspace: `cargo sweep --dry-run --time 7` would have removed 28 GiB from a 22 GB `target/`, i.e. effectively everything. That is not orphan sediment; almost all of it is third-party dependency rlibs compiled weeks ago and still very much in use, because the workspace's own artifacts are always freshly rebuilt. Age is a poor proxy for obsolete when your own crates churn daily and your dependencies never do.

```sh
cargo sweep --dry-run --time 7    # ALWAYS dry-run first; see above
cargo sweep --stamp && cargo build --tests && cargo sweep --file
```

The `--file` form claims to remove exactly what a build did not touch, but note it compares mtimes against the stamp, and an incremental build does not rewrite artifacts it did not rebuild — so it is only precise after a clean build, which defeats the purpose.

## Test fixtures

`stories/` is **gitignored** (commercial game files). Real-game integration tests must skip vacuously when their fixture is absent (see `any_v6_story_present()` in `crates/app/tests/suites/zmsd_screen_compliance.rs` for the CI-safe pattern). Freely redistributable fixtures live in `unit_tests/`. Git worktrees lack `stories/` — symlink it from the main checkout when smoke tests matter there.

**A disk image is a different release, not the same story on other media.** `stories/journey.z6` is release 83 / serial 890706; `Journey - The Quest Begins.adf` is release **30** / serial 890322, and the two differ in behaviour (r83 narrates through window 0, r30 through window 2 — which was the whole of SQ-0755). `InterpreterProfile::resolve` reads the medium, so "the Amiga build" means a different build of the game, not merely a different profile. Name the exact fixture and release in any finding, and when a defect is reported on a disk image, reproduce it on that image — a clean result off the bare story file proves nothing about it (SQ-0760). The release every medium in `stories/` carries is pinned in `crates/app/tests/suites/real_media_releases.rs` and tabulated in `docs/internals/interpreter.md`; drive the floppy there before claiming a suite covers "the Amiga profile".

## Architecture

Full detail in `docs/internals/architecture.md`; docs under `docs/internals/` track the code (README tracks the released build). Big picture:

- **`crates/zvm` / `gvm` / `scott`** — pure, headless VM cores (Z-machine, Glulx, Scott Adams). No I/O policy; they expose sessions the app drives. `zvm-cli` / `gvm-cli` / `scott-cli` are minimal terminal front-ends useful for debugging an engine without the TUI.
- **`crates/app`** — the lanthorn TUI. Talks to every engine through the engine-neutral `Engine` trait (`src/engine.rs`); `session.rs` (Z-machine), `glulx_session.rs`, and `scott_session.rs` adapt each VM into it. Glk exists only inside the Glulx adapter — it never leaks into shared app types.
- **`crates/mapper`** — the automap graph (rooms, exits, layout). Direction: map work moves off the main thread; only the story interpreter should run there.
- **`crates/blorb`**, **`crates/audio`** — resource-file parsing and sound playback.
- **Render pipeline** — `crates/app/src/render/`, entry `screen.rs`. Graphical v6 has two modes: **hybrid** (terminal cells for text, kitty graphics for art — the default; test this mode first) and **raster** (full-frame image). v6 geometry bugs are usually cell-quantization issues (art scaled by pixel, text placed by cell — watch for ceil-vs-round mismatches on shared boundaries).
  - **In hybrid, never rasterise what the game printed as a character.** That is what hybrid is *for*: text as text, art as art. A strip whose pixels the game's own paint runs fully explain must be drawn with glyphs. Rasterising a character costs alignment (a resampled edge meeting a font glyph on a shared boundary is exactly the ceil-vs-round trap above), costs crispness, and costs bandwidth — Journey ships four side rules as 8x900 and 16x900 RGBA bitmaps, ~192 KB per frame, to draw 200 `│`s, and the *same rule* is drawn as glyphs seven rows lower where it happens to cross the menu strip. Classify a strip by what is in it, not by where it sits: reserve raster for pixels the runs cannot account for, which is genuine artwork (Zork Zero's and Arthur's side columns) and nothing else. SQ-0750.
  - **Art density and text density are different facts, and on one machine they disagree.** The Version 6 cell is what the STORY IS TOLD (header `$26`/`$27`) and is the machine's, not the press's — 7x15 on a Macintosh, 8x16 everywhere else (SQ-0917). `art_scale` is how dense the ARTWORK is, and it is the archive's. Where those two differ, one native pixel means two different things in one frame:

    | press | picture space | `art_scale` | grid | one art px | one text px |
    |---|---|---|---|---|---|
    | Macintosh B/W | `Pic.data` 480x300 | (1, 1) | 68x20 | 1 native | 1 native |
    | Macintosh colour | `CPic.data` 320x200 | (2, 2) → 640x400 | 91x26 | **2 native** | 1 native |
    | Amiga | 320x200 | (2, 2) → 640x400 | 80x25 | 2 native | 1 native |

    The Amiga's 640x200 hires output mode is not a third number: the game's internal frame is 320x200, the display doubles it horizontally because a hires pixel is half as wide, and a modern square-pixel screen doubles it vertically too. `AM_XSIZ 640 / AM_YSIZ 200` in Infocom's own `amiga/yzip.h` describes the OUTPUT, not the coordinate space — read as an art scale it looks like (2,1) and is not (SQ-1023, discarded).

    **The consequence to remember: `set-v6-pixel-lock` quantizes to whole DEVICE pixels per ART pixel, so on any press where an art pixel is already two native pixels, a whole-art rung is a HALF-NATIVE one — and raster text is drawn at the native cell.** A 7-wide glyph at a 1.5 native scale gets 10.5 device pixels and its strokes alternate one and two. That is why the Macintosh colour press looks wrong at the half rungs and the B/W press never does, and why the fix is to skip those rungs rather than to change the face (SQ-1012, SQ-1024).

    **And a TYPEFACE is scaled by neither of those numbers directly, and not by the MACHINE either.** `zvm::interpreter::V6FaceSpace` states which space a face's bitmaps are authored in, and it is a property of **where the face came from**, not of the row: the Amiga draws its own RELEASES' faces in the PICTURE space, so a doubled press doubles them with it (Arthur's ten face rows are the twenty-row line the captures measure), while the SAME machine's system topaz is drawn in the 640x200 HIRES space and wants (1, 2) — 8x8 landing exactly on the declared 8x16 cell, which is the ten-of-twenty scanlines `machine-screenshots/amiga-shogun-game.png` shows over `Erasmus` (SQ-1053). The Macintosh answers `Native` both ways, painting text at one native pixel per face pixel however dense `CPic.data` is, which is why one number per machine looked sufficient until a second machine had a system face to read. `InterpreterProfile::release_face_space` / `system_face_space` are the only lookups, `V6FaceSpace::text_scale` the only arithmetic, and `native_font::face_space` the only place a provenance is turned into one of them; `TextFace` stores the answer and the declared cell, the advance table `zvm` wraps with and `render::bitfont`'s per-glyph blit all read it from there — so they cannot disagree. Scaling a face by `art_scale` instead declared Geneva 12's fifteen rows as thirty, and only on the COLOUR press: the B/W press is (1, 1) and cannot falsify it (SQ-1039).

- **Slash commands** — one registry, `slash::COMMANDS` (`src/slash.rs`), verb-noun names, keys bind to command strings. Add new commands there; there is no Command enum.
- **Config & styles** — `~/.lanthorn/config.toml` and `style.toml` are seeded as fully-commented templates (`src/config_template.rs`; uncommented section headers, `# key = default` lines). `write_config` writes only non-default values but always updates keys already present in the file. Per-game overrides are a bare-lines sidecar `<game_dir>/config.toml` (at most a few keys; absent key = inherit global) — never template it. Every new UI element must be styleable via a `style.toml` selector (ColorScheme field + `style.rs` selector + render apply); never hard-code styles.
- **Persistence** — two save families with distinct names: "Save State/Restore State" = engine-neutral host snapshots (save-anywhere, archive, auto-resume); `@save`/`@restore` = the game's own in-game Quetzal path. They must behave uniformly across engines. Pre-release: formats may break old files freely; no back-compat shims.
  - **Persist the recipe, not the result.** Nothing goes into the archive without either its regeneration inputs or a one-line comment saying why the derived artifact is authoritative. Quetzal saves no screen state by design — the standard assumes the *story* repaints after a restore, and a host Save State swaps memory under a game that never learns it happened, so everything the screen needs is ours to carry. Snapshotting an output (canvas PNGs) instead of its inputs (display list + palette) restores something that looks right and cannot be recomputed when the inputs change (SQ-0587/0588).
  - **The archive is backend- and terminal-neutral.** No cell coordinates, font metrics, or picker state in a save — v6 geometry is zvm native pixels, so a save moves between kitty/halfblocks/sixel and between terminal sizes. A restore reconciles the saved screen with the *current* pane (`reconcile_restored_screen_size`), because a restore into a different size is a resize the game never saw.

`machine-screenshots/` holds captures of the retail games running on **real
machines under emulation** (Amiga, C64/128, Apple IIe, …), committed because they
are the only thing in the repo that can falsify a question rather than an answer:
internal measurement tells you whether lanthorn does what you asked, never whether
you asked for the right thing. Reach for one before pinning a v6 geometry or colour
claim — and name the file in the finding, since a screenshot is a fixture with a
machine, a release and a moment in the game, exactly like a frame capture.

## Testing conventions

- Colour/render test areas pin **both** `honor_game_colours` modes (true is the shipped default and primary baseline); single-mode suites have masked regressions before.
- Falsify fixes: temporarily revert the fix and confirm the new test fails with the originally reported symptom before trusting it.
- **Restore tests must perturb before asserting.** Restore bugs surface one action *after* the restore, when the game next repaints, changes palette, splits, or resizes — asserting the frame immediately after a restore is when everything still looks correct. Restore, then make a move, then assert (`v6_restore_palette_replay.rs` is the pattern). Cover restoring into a *different* terminal size and a different graphics backend; both are common in the field and neither is visible to a same-session round-trip.
- Headless render harnesses live in the app integration tests (see `crates/app/tests/suites/v6_*.rs` for the pattern: drive a real story, render to a buffer, assert on cells/geometry).
- **Editor diagnostics that arrive while an agent is working are snapshots of an unfinished edit, not findings.** A half-written file genuinely has unbalanced parens, and a new call site genuinely outruns its `pub` export by a few seconds — both resolve themselves. `cargo check --all-targets` and the gate are the only authority; never act on a diagnostic without reproducing it there first. Multi-file lanes (render-path work especially) are quieter in a worktree, where the checkout the editor watches never sees the churn — symlink `stories/` into it or every real-game smoke skips vacuously into a false green.
- **Boot a harness the way `startup.rs` boots, or you measure a screen the app never draws.** The full chain is the profile (`InterpreterProfile::resolve`, from the medium the *mount* returned — not re-derived from the path) supplying palette, interpreter number and default colours, and the screen size `picts.std_window() → named archive → picts.native_std_window() → profile.std_window()` with `art_scale` alongside. Skip any step and the **game** lays its own windows out differently, so every rect measured afterwards is of a screen the player never sees, and the numbers look entirely self-consistent. Measured: `ring_scout` and `v6_side_border_tiling`'s `boot()` both omitted `native_std_window`, so Journey r77 and Arthur r63 — **560x384** presses — were booted at 640x400. That produced a fabricated Arthur frame ("a single illustration clear of both edges") which a whole quest was fixed and tested against, and hid two real defects for two rounds (SQ-0901, SQ-0883, SQ-0899). Print the profile, release and screen size the harness booted, and check them against a `/dump-windows` capture before trusting a measurement on disk media.
- **A frame is a fixture. Name the turn count and how you got there.** Real-game harnesses drive blank lines and single keys, which reaches an intro card and often nothing else — Arthur's ProDOS press renders identically at 6 and 40 keypresses because it never answers the restore question. SQ-0883 reproduces on the **menu** frame two turns in and was invisible in a case pinned to the gameplay frame four turns in. Put the turn count in the specimen table alongside the release, and give any case that depends on a frame's *shape* a non-vacuity guard asserting that shape — that guard is what caught the fabricated Arthur frame above.
- **Three render-testing layers; escalate only when the cheaper one can't explain the symptom.** Cell-buffer harnesses (`crates/app/tests/suites/v6_*.rs`) assert on lanthorn's INTERNAL model — always the first stop, but blind to a defect that's correct in the model and wrong on the user's screen. The emitted-stream harness (`crates/app/tests/pty_stream/`, SQ-0762; ad hoc via `cargo run -p app --example pty_capture`) runs the real binary under a pty and keeps every byte it emits — the pty must answer the terminal queries convincingly as kitty, or the capture silently measures the half-block backend and every number in it is worthless. Reach for it when the model looks right and the screen doesn't; it's the only layer that tells an image PLACEMENT apart from a background PAINTED into cells, indistinguishable on screen, different bugs. The placement oracle (`pty_stream/oracle.rs`, SQ-0764; dev-dep `qwertty-term-vt`) resolves those same bytes the way a real terminal does instead of through our hand-rolled decoder — reach for it when the stream also looks right and the screen is still wrong (placement lifetime, z-order, overlap, stale placements, unicode-placeholder continuation). It is a faithful **port** of Ghostty's core, not Ghostty itself — see `docs/internals/architecture.md` for its caveats (an id-encoding mismatch between the two decoders, the SQ-0772 image-coverage gap, and the libghostty-vt ground-truth escalation that exists but isn't built).
