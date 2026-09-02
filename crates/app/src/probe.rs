//! Fork the live game into a silent, disposable copy and ask it a question
//! (SQ-1121, and the seam SQ-0785 and SQ-1043 were scheduled to share).
//!
//! There was a first attempt at this — `return_probe.rs` on the abandoned commit
//! `5270882c` (2026-07-30), which probed the way back after a move to close
//! one-way gaps in the automap. It was never on a branch and never merged. Two
//! things from it are kept here because they were right: booting a shadow from
//! the STORY BYTES rather than cloning the engine, and restoring between every
//! candidate so probes stay independent. The rest of it was automapping wired
//! into a probe rather than a probe with a question in it, so this is the
//! generalisation SQ-0785's later note asked for rather than a re-landing.
//!
//! # Off the main thread, and booted the way the LIVE game boots (SQ-1124)
//!
//! Two things SQ-1121 got wrong, both fixed here and both worth stating because
//! the wrong version looked reasonable.
//!
//! **A budget is a cap on a stall, not a fix for one.** The shadow ran inline
//! under 400 ms, spent between the player's command and lanthorn's reply. Even
//! Zork I's measured 12 ms was main-thread time bought for an optional feature.
//! The shadow now lives on a worker thread ([`ShadowProbe`]); an offer is asked
//! for and collected a beat later, and the transcript takes the late arrival
//! through the same insert-above-prompt an assist has always used. An answer
//! that arrives after the player has typed again is stale and is dropped.
//!
//! **A shadow booted from the story bytes alone is not the same launch.** It got
//! an empty persistent store and an empty file VFS, so Counterfeit Monkey re-ran
//! the initialisation the live session skips — 2.1 s, measured — and the seam
//! wrote the story off. Both are now the live game's own, read-only. See
//! [`ShadowRecipe::store`].
//!
//! A **shadow** is a second [`Engine`] running the same story, driven from a
//! snapshot of the live one. Commands typed into it never reach the screen, the
//! filesystem, the sound card or the archive; when the answer has been read off
//! it, the shadow is restored back over and reused for the next question. The
//! live session is never touched — not saved, not restored, not stepped. That
//! separation is the point: restoring under a running game is the hazard
//! SQ-0587/0588 documented, because the game never learns it happened.
//!
//! # Why a shadow can answer anything at all
//!
//! [`Engine::save_state`] / [`Engine::restore_state`] are engine-neutral and
//! already in the trait — the host Save State family, not the game's own
//! `@save`. So "what would happen if I typed this?" is answerable by typing it
//! somewhere the answer costs nothing.
//!
//! # How this story says no, discovered rather than assumed
//!
//! The interesting question is almost never *did the parser understand this* —
//! that is a static fact about the dictionary, and [`crate::vocab`] answers it
//! without running anything. It is *did anything happen*, and every family of
//! game phrases its refusals differently (`[I don't know the word "x".]`, `You
//! can't see any such thing.`, `You use word(s) I don't know!`, `You don't have
//! that!`). A detector built on those strings is broken by the next game and
//! unusable outside English.
//!
//! So [`Refusals`] is **learned from the story**, in the shadow, by running
//! deliberate nonsense beside the real question and reading what comes back.
//! Two shapes of control, and each is only believed under a condition:
//!
//! * [`ProbeRun::refusal_from`] — a command the parser cannot have understood at
//!   all (a word this story's dictionary does not hold). Every sentence of the
//!   reply is a refusal.
//! * [`ProbeRun::refusal_from_pair`] — the same command twice with two different
//!   nouns in it. Believed **only as far as the two replies agree**, sentence by
//!   sentence, once their own nouns are struck out — which is what tells a
//!   generic refusal from two coincidentally similar successes, while still
//!   learning something when a daemon fires on one of the two and not the other.
//!
//! Both additionally require the control to have left the world unchanged
//! ([`WorldPrint`]): a control that moved an object *did* something, so whatever
//! it printed is not this story's way of saying no.
//!
//! # The controls belong to the ROOM, not to the session
//!
//! This is the thing that is easy to get wrong, and it was got wrong once here.
//! A refusal fingerprint learned at the start of a session and reused all game
//! is measuring the wrong room: Zork I answers `light rug` with `You don't have
//! that!` in the field and `You don't have the carpet.` in the living room, and
//! `light lamp` with `You don't have that!` outside the house and `(Taken) The
//! brass lantern is now on.` inside it. Same story, same command, different
//! answer — because scope is where the player is standing.
//!
//! So a caller runs its controls **in the same `run` as its questions**, from
//! the same snapshot, and reads the signature off that run. Nothing is cached
//! between turns.
//!
//! # What one question actually costs, and in which build (SQ-1249)
//!
//! The seam was reported at 4–10 s per vetted turn on heavy Inform 7 games.
//! [`ProbePhases`] exists because that had to be split before it could be
//! believed, and splitting it produced two answers rather than one.
//!
//! **The phase that dominates is `submit`** — the story running the commands we
//! asked about — at ~80% of the bill on every Glulx story measured. `boot` is
//! paid once a session and is a rounding error after it (SQ-1124 already moved
//! the shadow to one boot plus a restore per question, so there is no re-boot
//! and no replay of the turn history to remove); `restore` is ~10%; reading the
//! world after each command is ~5%. Nothing is left to optimise there that is
//! not "ask the story fewer questions", which is a change to what vetting MEANS.
//!
//! **And the 4–10 s was a DEBUG build.** `cargo run -p app --example
//! guidance_scan` prints the breakdown per story; the same scan, same fixtures,
//! same machine, one release flag apart (2026-09-02):
//!
//! | story | debug | release | worst single turn, release |
//! |---|---|---|---|
//! | weight-of-soul-public.gblorb | 11.64 s | **0.95 s** | 0.30 s |
//! | Sub_Rosa.gblorb | 11.89 s | **1.15 s** | 0.32 s |
//! | Alias 'The Magpie'.gblorb | 7.42 s | **0.56 s** | 0.09 s |
//! | Junior Arithmancer.gblorb | 3.92 s | **0.33 s** | 0.10 s |
//! | curses.z5 | 0.22 s | **0.06 s** | 0.01 s |
//!
//! Those are WORKER seconds across a whole scan, not a stall: the player's own
//! thread pays only the host snapshot in [`ShadowProbe::ask`], measured at 0.05 s
//! across the whole of weight-of-soul's scan. So the answer to "is this
//! affordable" is a build question, and a number taken under `cargo test` or
//! `cargo run` without `--release` is off by an order of magnitude. Quote the
//! release figure, and say which build any new one came from.
//!
//! # What it still cannot tell you
//!
//! A refusal that no control provokes reads as a success. And a game that
//! consumes randomness may answer the shadow and the live session differently:
//! a probe is evidence, never a guarantee.

use std::collections::BTreeSet;
use std::hash::{Hash, Hasher};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::engine::Engine;

/// The most commands one question will type into the shadow, whatever the
/// caller asks for. A story that answers instantly still cannot be walked
/// through a hundred candidates, and the worker must not be given work that
/// outlives the turn that wanted it by minutes. Sized for a question plus its
/// controls — [`crate::vocab`] asks three or four things and runs two controls
/// for each.
pub const MAX_PROBES: usize = 16;

// ── The recipe a shadow is built from ───────────────────────────────────────

/// Everything a silent copy of this story needs in order to exist: the story's
/// own bytes and the handful of boot facts that change how it runs.
///
/// One value rather than six parameters, deliberately (CLAUDE.md's refactoring
/// policy): a caller who supplied a subset would get a shadow that boots and
/// answers *plausibly* on a different machine than the live game, and nothing
/// downstream could tell.
///
/// **What it deliberately does NOT carry, and why.** A v6 launch resolves a
/// whole [`crate::machine_boot::MachineBoot`] — the screen the story is told it
/// has, the art scale, the character cell, §8.3.3's colour pair — and none of it
/// is here. Those facts change how a story is DRAWN; a shadow is only ever read
/// as text, and every comparison made against it (a candidate's reply against a
/// control's) is between two replies from the SAME shadow, so a shadow that
/// wraps differently from the live screen still answers the question asked of
/// it. If a caller ever needs a shadow's GEOMETRY, this is the value that has to
/// grow a `MachineBoot` — do that rather than adding the one field you happen to
/// want.
#[derive(Clone, Debug, Default)]
pub struct ShadowRecipe {
    /// The story file exactly as it was loaded, before any container was
    /// unwrapped — `hints::extract_story` does that again for the shadow.
    pub story_bytes: std::sync::Arc<Vec<u8>>,
    /// The live game's per-story directory, which the shadow reads and never
    /// writes (SQ-1124).
    ///
    /// This is the whole of the boot fix. A Glulx game's own fixed-name saves
    /// live here, and for Counterfeit Monkey one of them is the cache that lets
    /// it skip its initialisation — the "5.4s → 0.76s from the second launch"
    /// the CHANGELOG records. A shadow booted with no store re-runs that
    /// initialisation every session; a shadow pointed at the live store takes
    /// the same `@restore` path the live launch took. Read-only by construction
    /// ([`crate::glulx_session::GameStore`]), so the shadow can never write the
    /// cache it reads.
    ///
    /// Empty for a session with no per-story directory, which is every
    /// test-built recipe and costs only the old behaviour.
    pub store: std::path::PathBuf,
    /// The Glk file VFS the live session booted with, for the same reason: a
    /// game may read a cache out of it during initialisation (SQ-0290). Never
    /// written back — a shadow's VFS dies with it.
    pub vfs_bytes: std::sync::Arc<Vec<u8>>,
    /// Z-machine: whether the game may pick its own colours. Irrelevant to what
    /// a probe reads, but a boot fact, and a shadow that differs from the live
    /// game in any boot fact is a different game.
    pub honor_game_colours: bool,
    /// Z-machine header byte $1E.
    pub interpreter_number: Option<u8>,
    /// The seed the story's randomness starts from, so the shadow rolls the
    /// same dice the live session did.
    pub random_seed: Option<u32>,
    /// Glulx: whether the accelerated Glk functions are installed. Off would
    /// make the shadow's boot minutes long on Counterfeit Monkey.
    pub acceleration: bool,
    /// Glulx: the virtual screen the shadow lays its windows out on.
    pub screen: (u32, u32),
}

// ── What a probe hands back ─────────────────────────────────────────────────

/// A fingerprint of as much of the game world as an engine will show us:
/// where the player is, what is in the room, and what they are carrying.
///
/// Deliberately hashes and not descriptions — nothing reads them except to ask
/// whether they are the same as another print's. Every field is `None` for an
/// engine with no introspection, which is an honest "cannot tell", not "nothing
/// changed".
///
/// # Three facts, not one hash, because only two of them survive a save
/// (SQ-1248)
///
/// This was one `Option<u64>` over all of it, and that is not comparable
/// between two engines. [`Engine::current_location`] on a v4+ Z-machine story is
/// read off the **status line** — screen state, which no save carries and which
/// `GameSession::restore_state` deliberately blanks (SQ-0785, so a half-repainted
/// bar cannot name a plausible wrong room). A shadow therefore starts every
/// question with no status line at all, and whether it gets one back depends
/// entirely on whether the story repaints the whole bar during the probe turn:
///
/// | story | the shadow's bar after one probe turn | `current_location` |
/// |---|---|---|
/// | `vespers.z8` | repainted in full, `" Your Bedroom … Vespers "` | `Some` |
/// | `curses.z5` | only the fields that CHANGED, so the room row stays blank | **`None`** |
/// | `suvehnux.z5` | never split or drawn again after `Initialise` | **`None`** |
///
/// Folded into one hash, a location the live engine can read and the shadow
/// cannot made **every** step differ from the baseline. [`ProbeRun::inert`] then
/// called every control "did something", no refusal signature could be learned,
/// and every offer on those two stories came out unvetted though the probe ran
/// — while [`ProbeRun::did_something`] said yes to every candidate for the same
/// reason, which is the same defect pointing the other way.
///
/// So each fact is stored and compared on its own, and a fact only one side can
/// answer is not evidence of a change. That is [`Self::differs_from`]'s existing
/// rule ("two unreadable prints are not the same; they are not an answer")
/// applied per fact instead of to the bundle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WorldPrint {
    /// The player and what they are carrying — the object tree alone, so it is
    /// exactly what a restored save brings back and is always comparable.
    carried: Option<u64>,
    /// Where the player is, when the engine can say. Screen-derived above v3;
    /// see the type docs.
    here: Option<u16>,
    /// What is in that room besides the player. `None` whenever `here` is,
    /// because then the question was never asked.
    room: Option<u64>,
}

impl WorldPrint {
    /// Read the world as `engine` currently has it.
    pub fn of(engine: &dyn Engine) -> WorldPrint {
        let Some(intro) = engine.introspect() else { return WorldPrint::default() };
        let player = intro.player_object();
        let mut h = std::collections::hash_map::DefaultHasher::new();
        player.hash(&mut h);
        if let Some(p) = player {
            let mut v = intro.contents(p);
            v.sort();
            v.hash(&mut h);
        }
        let here = engine.current_location().map(|l| l.number);
        let room = here.map(|room| {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            let mut v = intro.room_objects_excluding(room, player);
            v.sort();
            v.hash(&mut h);
            h.finish()
        });
        WorldPrint { carried: Some(h.finish()), here, room }
    }

    /// True when some fact **both** prints can answer differs — a changed world.
    /// A fact only one of them holds is not an answer either way; see the type
    /// docs for the story that made that distinction load-bearing.
    pub fn differs_from(self, other: WorldPrint) -> bool {
        fn changed<T: PartialEq>(a: Option<T>, b: Option<T>) -> bool {
            matches!((a, b), (Some(a), Some(b)) if a != b)
        }
        changed(self.carried, other.carried)
            || changed(self.here, other.here)
            || changed(self.room, other.room)
    }

    /// A print with each fact stated outright, so a case can build the
    /// live-and-shadow pair SQ-1248 is about without two engines.
    #[cfg(test)]
    pub(crate) fn from_parts(
        carried: Option<u64>,
        here: Option<u16>,
        room: Option<u64>,
    ) -> WorldPrint {
        WorldPrint { carried, here, room }
    }
}

/// What one command did in the shadow.
#[derive(Clone, Debug)]
pub struct ProbeStep {
    /// The command as it was typed into the shadow.
    pub command: String,
    /// Everything the story printed in reply, and nothing else.
    pub reply: String,
    /// The room the shadow ended the command in, when the engine can say.
    pub location: Option<u16>,
    /// The world after the command.
    pub world: WorldPrint,
    /// The story ended.
    pub quit: bool,
    /// The command tried to reach outside the shadow — the game's own
    /// `@save`/`@restore`, or a Glk file prompt. It was refused and the step is
    /// worthless, but nothing escaped.
    pub escaped: bool,
}

/// One `run`: the world the questions were asked from, and what each answered.
#[derive(Clone, Debug)]
pub struct ProbeRun {
    /// The world at the snapshot every step started from.
    pub baseline: WorldPrint,
    /// One entry per command, in the order they were given.
    pub steps: Vec<ProbeStep>,
}

// ── This story's own signature of failure ───────────────────────────────────

/// The sentences this story prints when it has understood nothing and done
/// nothing — discovered, never assumed. See the module docs.
#[derive(Clone, Debug, Default)]
pub struct Refusals {
    sigs: BTreeSet<String>,
}

impl Refusals {
    /// True when `reply` to `command` is one of the refusals this story taught
    /// us. The command's own words are struck out of both sides, so the noun
    /// the sentence names does not have to match.
    ///
    /// Only the **first** sentence is compared, and that is load-bearing rather
    /// than an optimisation. Some engines put furniture in every reply — a Scott
    /// Adams turn ends with `Tell me what to do ?`, which is therefore inside
    /// the refusal a control taught us AND inside every success — so a rule of
    /// "any sentence matches" classifies every reply as a refusal and the offer
    /// falls silent on a whole engine. A refusal is what the story says FIRST;
    /// what follows it is a prompt, a daemon, or the lamp getting dimmer.
    pub fn says_no(&self, reply: &str, command: &str) -> bool {
        signature(reply, command).first().is_some_and(|s| self.sigs.contains(s))
    }

    /// Fold another reading of the same run in.
    pub fn merge(&mut self, other: Refusals) {
        self.sigs.extend(other.sigs);
    }

    /// True when nothing was learned — the controls taught nothing believable.
    /// A caller must not read silence here as "everything succeeded"; it means
    /// the run cannot answer.
    pub fn is_empty(&self) -> bool {
        self.sigs.is_empty()
    }

    /// The normalised sentences, for tests and diagnostics.
    pub fn sentences(&self) -> impl Iterator<Item = &str> {
        self.sigs.iter().map(String::as_str)
    }
}

impl ProbeRun {
    /// The step at `i` was a command the parser cannot have understood, so
    /// **everything** it printed is this story saying no.
    ///
    /// Empty when that step did something after all — moved an object, ended the
    /// story, reached for a file — because then its words describe an action,
    /// not a refusal.
    pub fn refusal_from(&self, i: usize) -> Refusals {
        let Some(step) = self.steps.get(i).filter(|s| self.inert(s)) else {
            return Refusals::default();
        };
        Refusals { sigs: signature(&step.reply, &step.command).into_iter().collect() }
    }

    /// Steps `a` and `b` are the same command carrying two different nouns.
    /// Their reply is a refusal **only if it opens with the same sentence** once
    /// each one's own noun is struck out — otherwise the two are describing two
    /// different things that happened, and neither is a refusal.
    ///
    /// **Only the sentences the two replies agree on, in order, are learned**
    /// (SQ-1248, the same rule [`WorldPrint::differs_from`] follows). A turn
    /// carries more than its refusal: a daemon fires on one control and not the
    /// other, and demanding that the WHOLE reply match then teaches nothing at
    /// all — `suvehnux.z5` answered two of its direction controls identically
    /// but appended `Something brushes past your foot.` to one of them, the pair
    /// taught nothing, and `fasten north` read as a success. The common prefix
    /// keeps every sentence both replies vouched for and drops the first
    /// divergence and everything after it, which is where a daemon lives.
    pub fn refusal_from_pair(&self, a: usize, b: usize) -> Refusals {
        let (Some(x), Some(y)) = (self.steps.get(a), self.steps.get(b)) else {
            return Refusals::default();
        };
        if !self.inert(x) || !self.inert(y) {
            return Refusals::default();
        }
        let sx = signature(&x.reply, &x.command);
        let sy = signature(&y.reply, &y.command);
        let agreed: Vec<String> =
            sx.into_iter().zip(sy).take_while(|(a, b)| a == b).map(|(a, _)| a).collect();
        Refusals { sigs: agreed.into_iter().collect() }
    }

    /// Did the step at `i` do anything, as far as this run can tell?
    ///
    /// A changed world settles it whatever was printed. An unchanged one settles
    /// nothing — `examine` and `look` legitimately change nothing — so the words
    /// decide, against a signature this same run discovered.
    pub fn did_something(&self, i: usize, refusals: &Refusals) -> bool {
        let Some(step) = self.steps.get(i) else { return false };
        // Ending the story is unambiguously something happening — a mistyped
        // `quit` is still a `quit`, and a player who typed `quti` meant it. Note
        // this is the OPPOSITE reading from [`Self::inert`]: a control that quit
        // teaches nothing, because the words it printed are a farewell rather
        // than a refusal.
        if step.quit {
            return true;
        }
        if step.escaped || step.reply.trim().is_empty() {
            return false;
        }
        step.world.differs_from(self.baseline) || !refusals.says_no(&step.reply, &step.command)
    }

    /// A step that reached nothing and changed nothing — the only kind whose
    /// words are safe to read as a refusal.
    fn inert(&self, step: &ProbeStep) -> bool {
        !step.escaped && !step.quit && !step.world.differs_from(self.baseline)
    }
}

/// One reply, reduced to the sentences that carry its *shape*: lowercased,
/// punctuation and digits dropped, and every word of the command that produced
/// it struck out — which is what makes `You can't see any lamp here!` and `You
/// can't see any sword here!` the same sentence, and what removes the quoted
/// word from `[I don't know the word "lanturn".]`.
fn signature(reply: &str, command: &str) -> Vec<String> {
    let typed: BTreeSet<String> = command
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    let mut out = Vec::new();
    for sentence in reply.split(['.', '!', '?', '\n']) {
        let words: Vec<String> = sentence
            .split(|c: char| !c.is_alphabetic())
            .filter(|w| !w.is_empty())
            .map(str::to_lowercase)
            .filter(|w| !typed.contains(w))
            .collect();
        if !words.is_empty() {
            out.push(words.join(" "));
        }
    }
    out
}

/// Where a probe's wall time actually went (SQ-1249).
///
/// A phase breakdown rather than one total, because the three phases have three
/// different fixes: a slow BOOT is the story's own startup and is paid once a
/// session, a slow RESTORE is the host snapshot being re-applied once per
/// command, and slow SUBMIT time is the story running the turns we asked for.
/// Guessing which one dominates is exactly the mistake this exists to stop —
/// the answer differed by an order of magnitude between engines.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProbePhases {
    /// Building the shadow engine: the story's own startup. Paid once per
    /// session (or again after a shadow the probe had to throw away), and zero
    /// on every other question — which is the fact a caller usually wants.
    pub boot: Duration,
    /// [`Engine::restore_state`], once before each command and once more to
    /// leave the shadow on the snapshot.
    pub restore: Duration,
    /// [`Engine::submit`]: the story running the command we asked about.
    pub submit: Duration,
    /// [`WorldPrint::of`]: reading the object tree after each command.
    pub world: Duration,
}

impl ProbePhases {
    /// Fold another breakdown in, phase by phase.
    pub fn add(&mut self, other: ProbePhases) {
        self.boot += other.boot;
        self.restore += other.restore;
        self.submit += other.submit;
        self.world += other.world;
    }
}

// ── The seam ────────────────────────────────────────────────────────────────

/// One question, on its way to the worker.
struct Job {
    token: u64,
    /// Shared, not cloned: a return search sends one direction at a time from
    /// ONE snapshot, and a per-job copy of the whole save blob was the
    /// player's thread paying twelve times for one moment (SQ-1177). The
    /// worker only ever reads it.
    save: std::sync::Arc<crate::engine::EngineSave>,
    baseline: WorldPrint,
    commands: Vec<String>,
}

/// One answer, on its way back.
///
/// Everything in it is plain data — `String`, `Vec<u8>`, numbers — which is what
/// lets the shadow stay on the worker thread and never cross back. No `Engine`
/// is `Send`, and none needs to be: the worker BUILDS its own from the recipe.
#[derive(Debug)]
pub struct Answer {
    /// Which question this answers. A caller that has moved on compares this
    /// against what it asked and drops anything it no longer wants.
    pub token: u64,
    /// What the shadow said, or `None` when it could not be asked at all.
    pub run: Option<ProbeRun>,
    /// The shadow could not be built, or would not take the live state. The seam
    /// switches itself off for the session rather than retrying every turn.
    broken: bool,
    /// Commands typed into the shadow for this question.
    probes: u32,
    /// Wall time the worker spent on it, the boot included.
    spent: Duration,
    /// Where that time went. See [`ProbePhases`].
    pub phases: ProbePhases,
}

/// One moment in the live game, ready to be asked questions about.
///
/// The host snapshot and the world print that go with it — two facts about the
/// same instant, which is why they travel as one value rather than as two
/// arguments a caller could pair up wrongly. Taken on the caller's thread,
/// because both are questions about the LIVE engine and it may not cross a
/// thread; everything after them is the worker's.
#[derive(Clone, Debug)]
pub struct ProbeSnapshot {
    save: std::sync::Arc<crate::engine::EngineSave>,
    baseline: WorldPrint,
}

/// The worker thread's end of the seam.
struct Worker {
    jobs: mpsc::Sender<Job>,
    answers: mpsc::Receiver<Answer>,
}

/// A silent copy of the live game, kept between questions, **on its own thread**.
///
/// Lives on [`crate::state::AppState`] because it is per-session state with a
/// lazy, expensive body: the shadow is booted the first time anything asks a
/// question and reused for every later one, so a story whose initialisation
/// costs millions of opcodes pays that once. A boot that fails disables the seam
/// for the session rather than being retried every turn.
///
/// # Why a thread, and what that changed (SQ-1124)
///
/// SQ-1121 ran the shadow inline, between the player's command and lanthorn's
/// reply, under a 400 ms budget. That budget was the wrong shape twice over: it
/// is a cap on a stall rather than a fix for one — even Zork I's measured 12 ms
/// is main-thread time bought for an optional feature — and it forced a
/// `too_slow` latch that wrote off the corpus's biggest game after a single
/// measurement.
///
/// So the shadow lives on a worker. [`ShadowProbe::ask`] hands it a snapshot and
/// a list of commands and returns immediately; [`ShadowProbe::poll`] collects
/// the answer whenever it arrives, which the event loop does every pass. Only
/// the story interpreter runs on the main thread, which is the direction the
/// project has been moving in anyway.
///
/// **Both budgets are gone with it**, and so is `too_slow`. A slow game simply
/// answers later, or not before the player types again — and then its answer is
/// stale and is dropped, which is the silence discipline this feature already
/// had rather than a new rule. What remains is [`MAX_PROBES`], which bounds the
/// WORK rather than the wait.
///
/// One question is in flight at a time. A second ask while the first is running
/// is refused (`None`), and the caller falls back to what it can say unvetted —
/// which bounds the queue at one and means a game that answers slowly degrades
/// to SQ-1041's behaviour instead of piling up turns of stale work.
#[derive(Default)]
pub struct ShadowProbe {
    recipe: Option<ShadowRecipe>,
    worker: Option<Worker>,
    /// The shadow could not be built; stop trying.
    broken: bool,
    /// The token of the question the worker is working on, if any.
    inflight: Option<u64>,
    /// Monotonic, so an answer can always be matched to its question even after
    /// the caller has stopped caring about it.
    next_token: u64,
    /// Commands typed into a shadow this session, and the time they took —
    /// the numbers `/info` would want, and the ones that say whether this is
    /// affordable on a given story.
    pub probes: u32,
    /// Total time spent inside the worker, boot included. Wall time on the
    /// WORKER, which is no longer time the player waited.
    pub spent: Duration,
    /// The same total, split by phase (SQ-1249).
    pub phases: ProbePhases,
}

impl std::fmt::Debug for ShadowProbe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShadowProbe")
            .field("armed", &self.recipe.is_some())
            .field("worker", &self.worker.is_some())
            .field("broken", &self.broken)
            .field("inflight", &self.inflight)
            .field("probes", &self.probes)
            .field("spent", &self.spent)
            .finish()
    }
}

impl ShadowProbe {
    /// Give the seam what it needs to build a shadow, and start the thread that
    /// will build it. Until this is called there is no probing — every
    /// test-built `AppState` is in that state, and so is a session whose story
    /// bytes were never kept.
    ///
    /// The thread is started here and the shadow is NOT: the worker blocks on
    /// its first job, so a session that never asks anything pays one parked
    /// thread and no initialisation at all.
    pub fn arm(&mut self, recipe: ShadowRecipe) {
        let (jobs, job_rx) = mpsc::channel::<Job>();
        let (answer_tx, answers) = mpsc::channel::<Answer>();
        let build = recipe.clone();
        // Detached on purpose. Joining would mean the app waits at exit for a
        // boot it no longer wants; dropping the sender ends the worker's `recv`
        // and it unwinds by itself, and a process exit ends it whatever it is
        // doing. Nothing it owns outlives it — the shadow's store is read-only
        // and its VFS is in memory.
        std::thread::Builder::new()
            .name("lanthorn-shadow".into())
            .spawn(move || shadow_worker(build, job_rx, answer_tx))
            .ok();
        self.recipe = Some(recipe);
        self.worker = Some(Worker { jobs, answers });
        self.broken = false;
        self.inflight = None;
    }

    /// True when a question could be asked — armed and not already given up on.
    pub fn is_armed(&self) -> bool {
        self.recipe.is_some() && self.worker.is_some() && !self.broken
    }

    /// True when a question is out with the worker and its answer has not been
    /// collected yet.
    pub fn is_busy(&self) -> bool {
        self.inflight.is_some()
    }

    /// Type each of `commands` into a silent copy of `live`, every one of them
    /// from the same snapshot — **later**, on the worker thread.
    ///
    /// Returns the token the answer will carry, or `None` when there is nothing
    /// to ask: unarmed, un-bootable, already busy, the live engine mid-`@save`,
    /// or a list longer than [`MAX_PROBES`]. That is "no answer", never "no".
    ///
    /// The snapshot and the world print are taken HERE, on the caller's thread,
    /// because both are questions about the LIVE engine and it may not cross a
    /// thread. Everything after them is the worker's.
    pub fn ask(&mut self, live: &dyn Engine, commands: &[String]) -> Option<u64> {
        // Refuse BEFORE snapshotting (SQ-1177). The checks are free and the
        // snapshot is the expensive half — `save_state` is ~102 ms on
        // Counterfeit Monkey in a debug build — and the busy case is exactly
        // the slow-game turn where paying it for a guaranteed `None` hurts
        // most. `ask_from` repeats the checks because it is public and has
        // callers of its own.
        if !self.is_armed() || self.is_busy() || commands.is_empty() || commands.len() > MAX_PROBES
        {
            return None;
        }
        self.ask_from(&self.snapshot(live)?, commands)
    }

    /// Take the state a question would be asked from, without asking one.
    ///
    /// Split out of [`ask`](Self::ask) for a caller that asks SEVERAL questions
    /// about one moment (SQ-0785): a return search sends one direction at a time
    /// so that every answered attempt is durable, and re-snapshotting per attempt
    /// would charge the player's thread for each of them. Measured on Counterfeit
    /// Monkey in a debug build, one `save_state` is **102 ms** — twelve of those
    /// is the kind of main-thread cost SQ-1124 exists to have removed, and one is
    /// not.
    ///
    /// The state does go stale as the player keeps playing, and that is the right
    /// trade rather than a defect to fix: a search reads a snapshot taken the
    /// moment they arrived, and what it can then get wrong is MISSING a passage
    /// something they did since has opened. It cannot invent one.
    ///
    /// `None` for a suspended VM: snapshotting one would capture it
    /// mid-file-operation, and the shadow would resume into an I/O request
    /// nobody can answer.
    pub fn snapshot(&self, live: &dyn Engine) -> Option<ProbeSnapshot> {
        self.snapshot_from(live, || std::sync::Arc::new(live.save_state()))
    }

    /// [`snapshot`](Self::snapshot), from a host save something else already
    /// paid for — the turn path takes ONE `save_state` per turn and shares it
    /// between history, the auto-save and this seam (SQ-1178's `TurnSave`).
    ///
    /// `save` is a closure rather than a value so the guards still come first:
    /// an unarmed seam or a suspended VM must refuse before anything pays for
    /// a snapshot, exactly as [`ask`](Self::ask) refuses before taking one
    /// (SQ-1177). A caller holding an already-materialised Arc loses nothing —
    /// its closure is a clone.
    pub fn snapshot_from(
        &self,
        live: &dyn Engine,
        save: impl FnOnce() -> std::sync::Arc<crate::engine::EngineSave>,
    ) -> Option<ProbeSnapshot> {
        if !self.is_armed() || live.is_saveload_pending() {
            return None;
        }
        Some(ProbeSnapshot { save: save(), baseline: WorldPrint::of(live) })
    }

    /// [`ask`](Self::ask), from a snapshot already taken. See
    /// [`snapshot`](Self::snapshot).
    pub fn ask_from(&mut self, from: &ProbeSnapshot, commands: &[String]) -> Option<u64> {
        if !self.is_armed() || self.is_busy() || commands.is_empty() || commands.len() > MAX_PROBES
        {
            return None;
        }
        let token = self.next_token.wrapping_add(1);
        let job = Job {
            token,
            save: std::sync::Arc::clone(&from.save),
            baseline: from.baseline,
            commands: commands.to_vec(),
        };
        self.worker.as_ref()?.jobs.send(job).ok()?;
        self.next_token = token;
        self.inflight = Some(token);
        Some(token)
    }

    /// Collect an answer if one has arrived. Never blocks; call it every pass of
    /// the event loop.
    pub fn poll(&mut self) -> Option<Answer> {
        let answer = self.worker.as_ref()?.answers.try_recv().ok()?;
        self.settled(&answer);
        Some(answer)
    }

    /// Block until the question in flight answers. **Not for the event loop** —
    /// this is the measurement and test path, and the one place a caller
    /// deliberately wants the old synchronous cost so it can print it.
    pub fn settle(&mut self) -> Option<Answer> {
        self.inflight?;
        let answer = self.worker.as_ref()?.answers.recv().ok()?;
        self.settled(&answer);
        Some(answer)
    }

    /// Fold a collected answer's bookkeeping back in.
    fn settled(&mut self, answer: &Answer) {
        self.probes += answer.probes;
        self.spent += answer.spent;
        self.phases.add(answer.phases);
        if answer.broken {
            self.broken = true;
        }
        if self.inflight == Some(answer.token) {
            self.inflight = None;
        }
    }

    /// Ask and wait — the synchronous shape, kept for measurement harnesses and
    /// for tests that want one answer and no event loop.
    ///
    /// Nothing in the app calls this: an offer asks with [`ask`](Self::ask) and
    /// collects with [`poll`](Self::poll), which is the entire point of SQ-1124.
    pub fn run(&mut self, live: &dyn Engine, commands: &[String]) -> Option<ProbeRun> {
        self.ask(live, commands)?;
        self.settle()?.run
    }
}

/// The worker thread: owns the shadow for the life of the session, and is the
/// only thing that ever touches it.
///
/// Exits when the sender is dropped, which is what `arm` (re-arming) and dropping
/// the [`ShadowProbe`] both do.
fn shadow_worker(recipe: ShadowRecipe, jobs: mpsc::Receiver<Job>, answers: mpsc::Sender<Answer>) {
    let mut shadow: Option<Box<dyn Engine>> = None;
    while let Ok(job) = jobs.recv() {
        let started = Instant::now();
        let mut probes = 0u32;
        let mut phases = ProbePhases::default();
        let answer = match serve(&recipe, &mut shadow, &job, &mut probes, &mut phases) {
            Ok(run) => Answer {
                token: job.token,
                run,
                broken: false,
                probes,
                spent: started.elapsed(),
                phases,
            },
            Err(()) => {
                shadow = None;
                Answer {
                    token: job.token,
                    run: None,
                    broken: true,
                    probes,
                    spent: started.elapsed(),
                    phases,
                }
            }
        };
        if answers.send(answer).is_err() {
            return; // nobody is listening any more
        }
    }
}

/// Run one job in the shadow, booting it first if this is the first question.
///
/// `Err(())` is "there is no usable shadow" — the boot failed, or it would not
/// take the live state — which disables the seam. `Ok(None)` is "nothing to
/// report", which does not.
fn serve(
    recipe: &ShadowRecipe,
    shadow: &mut Option<Box<dyn Engine>>,
    job: &Job,
    probes: &mut u32,
    phases: &mut ProbePhases,
) -> Result<Option<ProbeRun>, ()> {
    if shadow.is_none() {
        let t = Instant::now();
        let booted = boot_shadow(recipe).map_err(|_| ())?;
        phases.boot += t.elapsed();
        *shadow = Some(booted);
    }
    let engine = shadow.as_mut().ok_or(())?;

    let mut steps = Vec::with_capacity(job.commands.len());
    for command in &job.commands {
        let t = Instant::now();
        let restored = engine.restore_state(&job.save);
        phases.restore += t.elapsed();
        if restored.is_err() {
            // A shadow that will not take the live state is no shadow.
            return Err(());
        }
        let _ = engine.take_transcript();
        let _ = engine.take_transcript_elems();
        let t = Instant::now();
        let result = engine.submit(command);
        phases.submit += t.elapsed();
        *probes += 1;
        // ISOLATION. Nothing typed in here may reach a file. A game that
        // suspends for its own `@save`/`@restore`, or asks Glk for a
        // filename, is answered "that failed" so the VM unwinds inside the
        // shadow, and the step is thrown away.
        let escaped = result.pending_io.is_some() || engine.pending_filename().is_some();
        if escaped {
            unwind_io(engine.as_mut(), result.pending_io);
        }
        let t = Instant::now();
        let world = WorldPrint::of(&**engine);
        phases.world += t.elapsed();
        steps.push(ProbeStep {
            command: command.clone(),
            reply: result.transcript.clone(),
            location: result.location.as_ref().map(|l| l.number),
            world,
            quit: result.quit,
            escaped,
        });
    }

    // A shadow the probe QUIT is dead, and restoring memory under it does
    // not bring it back — the next `submit` would return nothing and the
    // run after this one would silently read every reply as empty. Throw it
    // away and let the next question boot a fresh one. (Found by `quti` on
    // a Scott story: the shadow quit, and the very next offer went unvetted
    // with no sign anything was wrong.)
    if steps.iter().any(|s| s.quit) {
        *shadow = None;
    } else {
        // Otherwise leave the shadow on the snapshot rather than on the last
        // probe's aftermath, so a shadow that is never asked again is
        // holding a state the live game actually reached.
        let t = Instant::now();
        let _ = engine.restore_state(&job.save);
        phases.restore += t.elapsed();
        let _ = engine.take_transcript();
    }

    Ok((!steps.is_empty()).then_some(ProbeRun { baseline: job.baseline, steps }))
}


/// Answer whatever host I/O the shadow suspended on with a failure, so the VM
/// resumes and unwinds *inside* the shadow instead of sitting suspended.
///
/// This is the one place a probe could have reached the filesystem, so it is
/// answered rather than merely detected: an in-game `@save` is told the write
/// failed and an in-game `@restore` that the player cancelled, and a Glk
/// `create_by_prompt` gets no filename. The game then prints its own "Failed."
/// and carries on, in a copy that is about to be overwritten anyway.
fn unwind_io(shadow: &mut dyn Engine, io: Option<crate::session::PendingIo>) {
    if shadow.pending_filename().is_some() {
        let _ = shadow.resume_filename(None);
    }
    match io {
        Some(crate::session::PendingIo::Save) => {
            let _ = shadow.resume_save(false);
        }
        Some(crate::session::PendingIo::Restore) => {
            let _ = shadow.resume_restore(None);
        }
        None => {}
    }
}

/// Boot a silent, disposable engine for the same story.
///
/// Everything that could WRITE outside the process is off: no sound, no
/// graphics, no Blorb, and a **read-only** persistent store, so the game's own
/// fixed-name Glk saves are answered "that failed" and nothing the shadow does
/// survives it.
///
/// What it may READ is the live game's own persistent data, and that is
/// SQ-1124's boot fix (see [`ShadowRecipe::store`]). SQ-1121 booted with an
/// empty store and an empty VFS, which is not "isolated" so much as "a different
/// launch of the story": Counterfeit Monkey then re-ran the initialisation the
/// live session had already skipped, and the seam wrote the whole story off as
/// too slow to probe. Reading the store costs nothing and is not a leak — the
/// data is this very game's, and the shadow is about to have the live snapshot
/// restored over it anyway.
fn boot_shadow(recipe: &ShadowRecipe) -> Result<Box<dyn Engine>, String> {
    match crate::hints::extract_story(recipe.story_bytes.as_ref().clone())
        .map_err(|e| e.to_string())?
    {
        crate::hints::LoadedStory::ZCode(bytes) => {
            let s = crate::session::GameSession::new_with_trace(
                bytes,
                recipe.honor_game_colours,
                false, // sound unavailable: the story is told there is no sound card
                recipe.interpreter_number,
                false,
                Vec::new(),
                None,
                None,
                None,
            )
            .map_err(|e| format!("{e:?}"))?;
            Ok(Box::new(s))
        }
        crate::hints::LoadedStory::Glulx(bytes) => {
            let s = crate::glulx_session::GlulxSession::new_shadow(
                recipe.store.clone(),
                bytes,
                recipe.screen.0,
                recipe.screen.1,
                recipe.acceleration,
                &recipe.vfs_bytes,
                recipe.random_seed,
            )
            .map_err(|e| format!("{e:?}"))?;
            Ok(Box::new(s))
        }
        crate::hints::LoadedStory::Scott(bytes) => {
            let s = crate::scott_session::ScottSession::new_with_trace(
                bytes,
                None,
                false,
                recipe.random_seed,
            )?;
            Ok(Box::new(s))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signature_strikes_out_the_words_the_command_supplied() {
        assert_eq!(
            signature("You can't see any lamp here!", "light lamp"),
            vec!["you can t see any here"]
        );
        assert_eq!(
            signature("You can't see any sword here!", "light sword"),
            vec!["you can t see any here"],
            "two nouns, one sentence — which is what makes the fingerprint portable"
        );
    }

    #[test]
    fn a_quoted_word_leaves_with_the_command_that_carried_it() {
        assert_eq!(
            signature("[I don't know the word \"lanturn\".]", "take lanturn"),
            vec!["i don t know the word"]
        );
    }

    #[test]
    fn refusals_recognise_the_shape_and_not_the_noun() {
        let mut r = Refusals {
            sigs: signature("You can't see any zzqx here!", "take zzqx").into_iter().collect(),
        };
        assert!(r.says_no("You can't see any lamp here!", "light lamp"));
        assert!(!r.says_no("The brass lantern is now on.", "light lamp"));
        assert!(!r.is_empty());
        r.merge(Refusals {
            sigs: signature("You don't have that!", "light lamp").into_iter().collect(),
        });
        assert!(r.says_no("You don't have that!", "light sword"), "merged readings both count");
    }

    /// A Scott Adams turn ends with `Tell me what to do ?`, so that sentence is
    /// inside every reply the engine ever gives — the refusals a control teaches
    /// AND the successes. Matching on any sentence would silence the offer on
    /// the whole engine; matching on the first does not.
    #[test]
    fn a_prompt_that_rides_every_reply_does_not_make_every_reply_a_refusal() {
        let refused = "You use word(s) I don't know!\n\nTell me what to do ?";
        let worked = "OK.\n\nTell me what to do ?";
        let r = Refusals { sigs: signature(refused, "zqxwvj").into_iter().collect() };
        assert!(
            r.sentences().any(|s| s == "tell me what to do"),
            "the prompt IS in the signature — that is the situation being handled"
        );
        assert!(r.says_no(refused, "zqxwvj"));
        assert!(!r.says_no(worked, "take lamp"), "a success wearing the same prompt");
    }

    #[test]
    fn an_unlearned_signature_says_nothing_rather_than_no() {
        let r = Refusals::default();
        assert!(r.is_empty());
        assert!(!r.says_no("You can't see any lamp here!", "light lamp"));
    }

    #[test]
    fn an_unreadable_world_is_not_an_unchanged_one() {
        let blind = WorldPrint::default();
        let one = WorldPrint::from_parts(Some(1), None, None);
        assert!(!blind.differs_from(blind));
        assert!(!blind.differs_from(one));
        assert!(one.differs_from(WorldPrint::from_parts(Some(2), None, None)));
        assert!(!one.differs_from(one));
    }

    /// **SQ-1248.** A shadow restored from a save has no status line, so on a
    /// story that does not repaint the whole bar every turn its
    /// `current_location` is `None` while the LIVE engine's is `Some` — and the
    /// room contents hang off that same answer. Neither is evidence that
    /// anything moved, and folded into one hash both of them said it had.
    ///
    /// Falsify by folding the three fields back into one hash: every assertion
    /// below flips.
    #[test]
    fn a_fact_only_one_side_can_answer_is_not_a_changed_world() {
        let live = WorldPrint::from_parts(Some(7), Some(35), Some(99));
        let shadow = WorldPrint::from_parts(Some(7), None, None);
        assert!(
            !shadow.differs_from(live),
            "the shadow could not read a status line; that is not a move"
        );
        assert!(!live.differs_from(shadow), "and the comparison is symmetric");
        // What the two CAN both answer still decides.
        assert!(
            WorldPrint::from_parts(Some(8), None, None).differs_from(live),
            "the inventory changed, and both sides can see it"
        );
        assert!(
            WorldPrint::from_parts(Some(7), Some(36), None).differs_from(live),
            "both sides named a room and they are different rooms"
        );
        assert!(
            WorldPrint::from_parts(Some(7), Some(35), Some(100)).differs_from(live),
            "same room, different contents"
        );
    }

    /// **SQ-1248's second half.** A turn carries more than its refusal. Suvehnux
    /// answers two direction controls identically and then appends `Something
    /// brushes past your foot.` to whichever one the daemon fires on; demanding
    /// the WHOLE reply match made that pair teach nothing, and the candidate
    /// read as a success. The sentences the two agree on are still evidence.
    ///
    /// Falsify by restoring the whole-reply equality: the first assertion fails.
    #[test]
    fn a_daemon_on_one_control_still_leaves_the_pair_something_to_teach() {
        let step = |command: &str, reply: &str| ProbeStep {
            command: command.to_string(),
            reply: reply.to_string(),
            location: None,
            world: WorldPrint::from_parts(Some(7), None, None),
            quit: false,
            escaped: false,
        };
        let run = ProbeRun {
            baseline: WorldPrint::from_parts(Some(7), None, None),
            steps: vec![
                step(
                    "fasten east",
                    "There is no obvious way to do that.\n\nSomething brushes past your foot.",
                ),
                step("fasten south", "There is no obvious way to do that."),
                step("fasten hither", "The wind takes it clean out of your hands."),
            ],
        };
        let agreed = run.refusal_from_pair(0, 1);
        assert!(
            agreed.says_no("There is no obvious way to do that.", "fasten north"),
            "the sentence BOTH controls printed taught nothing: {:?}",
            agreed.sentences().collect::<Vec<_>>()
        );
        assert!(
            !agreed.says_no("Something brushes past your foot.", "fasten north"),
            "the daemon line was learned as a refusal, though only one control showed it"
        );
        assert!(
            run.refusal_from_pair(0, 2).is_empty(),
            "two replies that disagree from their first sentence still teach nothing"
        );
    }

    /// The same defect where it actually bit: a control whose reply is this
    /// story's refusal teaches nothing at all if the print calls it a move.
    #[test]
    fn a_control_the_shadow_cannot_place_still_teaches_its_refusal() {
        let step = |command: &str, reply: &str| ProbeStep {
            command: command.to_string(),
            reply: reply.to_string(),
            location: None,
            // The shadow's own print: it knows the tree, not the screen.
            world: WorldPrint::from_parts(Some(7), None, None),
            quit: false,
            escaped: false,
        };
        let run = ProbeRun {
            // The LIVE engine's, taken with a status line on screen.
            baseline: WorldPrint::from_parts(Some(7), Some(35), Some(99)),
            steps: vec![
                step("zqxwvj", "That's not a verb I recognise."),
                step("examine ace", "You can't see any such thing."),
                step("examine adamant", "You can't see any such thing."),
                step("examine hinged", "You see nothing special about the hinged trapdoor."),
            ],
        };
        let mut refusals = run.refusal_from(0);
        assert!(!refusals.is_empty(), "the unknown-word control taught nothing");
        refusals.merge(run.refusal_from_pair(1, 2));
        assert!(
            refusals.says_no("You can't see any such thing.", "examine lamp"),
            "the absent-noun pair taught nothing"
        );
        assert!(run.did_something(3, &refusals), "a real description read as a refusal");
        assert!(!run.did_something(1, &refusals), "a refusal read as something happening");
    }

    /// A stand-in whose only job is to count what a refusal costs. Everything
    /// a refused `ask` has no business touching is `unreachable!`.
    struct CountingEngine {
        saves: std::cell::Cell<u32>,
    }

    impl Engine for CountingEngine {
        fn submit(&mut self, _command: &str) -> crate::session::TurnResult {
            unreachable!("a refused ask types nothing")
        }
        fn submit_key(&mut self, _key: crate::engine::KeyInput) -> Option<crate::session::TurnResult> {
            unreachable!("a refused ask types nothing")
        }
        fn take_transcript(&mut self) -> String {
            String::new()
        }
        fn drain_screen_clear(&mut self) -> bool {
            false
        }
        fn pending_input(&self) -> crate::session::InputKind {
            crate::session::InputKind::Line
        }
        fn resume_save(&mut self, _wrote_ok: bool) -> crate::session::TurnResult {
            unreachable!("not exercised by this test")
        }
        fn resume_restore(&mut self, _data: Option<&[u8]>) -> crate::session::TurnResult {
            unreachable!("not exercised by this test")
        }
        fn has_quit(&self) -> bool {
            false
        }
        fn screen(&self) -> crate::engine::ScreenModel {
            unreachable!("not exercised by this test")
        }
        fn save_state(&self) -> crate::engine::EngineSave {
            self.saves.set(self.saves.get() + 1);
            crate::engine::EngineSave::new("mock", 1, Vec::new())
        }
        fn restore_state(
            &mut self,
            _save: &crate::engine::EngineSave,
        ) -> Result<(), crate::engine::EngineError> {
            unreachable!("not exercised by this test")
        }
        fn restore_game_save(&mut self, _bytes: &[u8]) -> Result<(), crate::engine::EngineError> {
            unreachable!("not exercised by this test")
        }
        fn aux_data(&self) -> &std::collections::BTreeMap<String, Vec<u8>> {
            unreachable!("not exercised by this test")
        }
        fn set_aux_data(&mut self, _data: std::collections::BTreeMap<String, Vec<u8>>) {
            unreachable!("not exercised by this test")
        }
        fn aux_dirty(&self) -> bool {
            false
        }
        fn clear_aux_dirty(&mut self) {}
        fn current_location(&self) -> Option<crate::engine::LocationInfo> {
            None
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
    }

    /// SQ-1177: a busy probe must say no BEFORE paying for the snapshot. The
    /// busy turn is exactly the slow-game turn — the previous question is
    /// still running because the story answers slowly — so charging the
    /// player's thread a full `save_state` for a guaranteed `None` was the
    /// worst possible moment for it.
    #[test]
    fn a_busy_probe_refuses_before_paying_for_a_snapshot() {
        let mut p = ShadowProbe::default();
        p.arm(ShadowRecipe::default());
        assert!(p.is_armed(), "armed, so busyness is the only refusal in play");
        p.inflight = Some(1); // a question is out with the worker
        assert!(p.is_busy());
        let live = CountingEngine { saves: std::cell::Cell::new(0) };
        assert_eq!(p.ask(&live, &["north".to_string()]), None, "busy refuses");
        assert_eq!(live.saves.get(), 0, "and the refusal must cost no save_state");
    }

    /// The guards hold on the shared-save seam too: an unarmed probe refuses
    /// before materialising the snapshot it was offered (SQ-1178).
    #[test]
    fn snapshot_from_refuses_before_materialising_the_save() {
        let p = ShadowProbe::default();
        let live = CountingEngine { saves: std::cell::Cell::new(0) };
        let took = std::cell::Cell::new(false);
        let snap = p.snapshot_from(&live, || {
            took.set(true);
            std::sync::Arc::new(live.save_state())
        });
        assert!(snap.is_none(), "unarmed refuses");
        assert!(!took.get(), "and never asked for the save it would not use");
    }

    /// A shadow that only ever answers a question by RESTORING the live
    /// snapshot over itself — never by re-booting the story, never by replaying
    /// the turns that reached this moment.
    ///
    /// Counts what a question actually costs it, because SQ-1249 went looking
    /// for a boot or a replay hiding inside the per-turn bill and had to be able
    /// to prove there was neither. `save_state` is `unreachable!`: the shadow is
    /// never asked for its own state, only handed the live game's.
    #[derive(Default)]
    struct ShadowStub {
        restores: u32,
        typed: Vec<String>,
    }

    impl Engine for ShadowStub {
        fn submit(&mut self, command: &str) -> crate::session::TurnResult {
            self.typed.push(command.to_string());
            crate::session::TurnResult::default()
        }
        fn submit_key(&mut self, _key: crate::engine::KeyInput) -> Option<crate::session::TurnResult> {
            unreachable!("a probe types commands, not keys")
        }
        fn take_transcript(&mut self) -> String {
            String::new()
        }
        fn drain_screen_clear(&mut self) -> bool {
            false
        }
        fn pending_input(&self) -> crate::session::InputKind {
            crate::session::InputKind::Line
        }
        fn resume_save(&mut self, _wrote_ok: bool) -> crate::session::TurnResult {
            unreachable!("nothing here suspends")
        }
        fn resume_restore(&mut self, _data: Option<&[u8]>) -> crate::session::TurnResult {
            unreachable!("nothing here suspends")
        }
        fn has_quit(&self) -> bool {
            false
        }
        fn screen(&self) -> crate::engine::ScreenModel {
            unreachable!("a shadow is never drawn")
        }
        fn save_state(&self) -> crate::engine::EngineSave {
            unreachable!("the shadow's own state is never wanted")
        }
        fn restore_state(
            &mut self,
            _save: &crate::engine::EngineSave,
        ) -> Result<(), crate::engine::EngineError> {
            self.restores += 1;
            Ok(())
        }
        fn restore_game_save(&mut self, _bytes: &[u8]) -> Result<(), crate::engine::EngineError> {
            unreachable!("not exercised by this test")
        }
        fn aux_data(&self) -> &std::collections::BTreeMap<String, Vec<u8>> {
            unreachable!("not exercised by this test")
        }
        fn set_aux_data(&mut self, _data: std::collections::BTreeMap<String, Vec<u8>>) {
            unreachable!("not exercised by this test")
        }
        fn aux_dirty(&self) -> bool {
            false
        }
        fn clear_aux_dirty(&mut self) {}
        fn current_location(&self) -> Option<crate::engine::LocationInfo> {
            None
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
    }

    fn stub_job(token: u64, commands: &[&str]) -> Job {
        Job {
            token,
            save: std::sync::Arc::new(crate::engine::EngineSave::new("mock", 1, Vec::new())),
            baseline: WorldPrint(None),
            commands: commands.iter().map(|c| c.to_string()).collect(),
        }
    }

    /// SQ-1249: the per-turn bill is the story running the commands we asked
    /// about, and nothing else. A question costs NO boot — the shadow is built
    /// once and every question after that restores the live host snapshot over
    /// it — and NO replay: the only things typed into the shadow are the
    /// commands the caller named.
    ///
    /// The falsification is built in. `ShadowRecipe::default()` carries no story
    /// bytes, so the boot path cannot succeed: the third arm below forces the
    /// shadow away and shows that a seam which re-booted per question would fail
    /// outright here rather than quietly charging for it.
    #[test]
    fn a_question_restores_the_live_snapshot_and_neither_boots_nor_replays() {
        let recipe = ShadowRecipe::default();
        let mut shadow: Option<Box<dyn Engine>> = Some(Box::new(ShadowStub::default()));
        let mut phases = ProbePhases::default();
        let mut probes = 0u32;

        let first = serve(&recipe, &mut shadow, &stub_job(1, &["light lamp", "light zzqx"]), &mut probes, &mut phases)
            .expect("an already-built shadow answers")
            .expect("two commands is two steps");
        assert_eq!(first.steps.len(), 2);
        assert!(phases.boot.is_zero(), "an existing shadow is not re-booted");

        // A second question a turn later, on the same worker and the same shadow.
        let second = serve(&recipe, &mut shadow, &stub_job(2, &["open door"]), &mut probes, &mut phases)
            .expect("and answers again")
            .expect("one command is one step");
        assert_eq!(second.steps.len(), 1);
        assert_eq!(probes, 3, "three commands typed across the two questions");
        assert!(phases.boot.is_zero(), "and still no boot on the second question");

        let stub = shadow
            .as_ref()
            .expect("the shadow survives an answered question")
            .as_any()
            .downcast_ref::<ShadowStub>()
            .expect("the same stub throughout");
        assert_eq!(
            stub.typed,
            vec!["light lamp", "light zzqx", "open door"],
            "only the commands asked about — no turn history is replayed into the shadow"
        );
        assert_eq!(
            stub.restores, 5,
            "one restore per command, plus one per question to leave the shadow on the snapshot"
        );

        // Falsification: force the pre-SQ-1124 shape, where a question with no
        // standing shadow has to boot one. With no story bytes that is a hard
        // failure, which is exactly how we know the arms above never took it.
        let mut none: Option<Box<dyn Engine>> = None;
        assert!(
            serve(&recipe, &mut none, &stub_job(3, &["look"]), &mut 0, &mut ProbePhases::default())
                .is_err(),
            "a question that has to boot goes down the boot path"
        );
    }

    #[test]
    fn an_unarmed_probe_asks_nothing() {
        let p = ShadowProbe::default();
        assert!(!p.is_armed());
        assert_eq!(p.probes, 0, "an unarmed probe has typed nothing");
    }
}
