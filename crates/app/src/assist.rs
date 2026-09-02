//! Lanthorn's Guiding Light — the voice every assist speaks in (SQ-1045).
//!
//! An **assist** is lanthorn helping the player *play*: the game's own words when
//! the parser rejects theirs, a completed noun, a caution before a move that
//! cannot be taken back, a pointer at the hints that exist. It is not a report of
//! something lanthorn did (that is [`TranscriptKind::Meta`]) and not a fault
//! (that is [`TranscriptKind::Warning`]). Ask which of the three a line is before
//! reaching for this module: *does it help the player play?*
//!
//! # The register
//!
//! A lanthorn is a lantern, and the lamp is the most iconic object in the genre,
//! so the helper reads as the interpreter holding up a light. But atmosphere must
//! not cost **attribution**, and attribution is the whole point:
//!
//! > Infocom's parser already speaks in brackets — `[I don't know the word
//! > "illuminate".]` — and a helper writing in the same register, in the same
//! > stream, is confusing. It is worse than confusing when the helper is
//! > **wrong**, because the player attributes our mistake to the game.
//!
//! # Attribution is split by SURFACE, because nobody sees two of them at once
//!
//! Every assist line used to begin with the words `Lanthorn: `, on the argument
//! that the words are the only carrier that survives a copy-paste, a saved
//! transcript and a screen reader. The argument about *files* was right and the
//! conclusion about the *screen* was wrong: on screen the marker is ten columns
//! of furniture repeated on every line of a forty-column pane, saying a thing the
//! eye can be told once. So the identifier is chosen per surface, and no reader
//! is ever on two surfaces at once:
//!
//! 1. **On screen: the mark.** The gutter glyph *is* the light — a filled dot,
//!    [`crate::symbols::SymbolSet::assist_gutter`], in the terminal's yellow slot
//!    (`transcript_assist` / `transcript_assist_caution`, both parented on
//!    `alert`, the caution tone bold). No prefix rides the text. The glyph is the
//!    user's: `[symbols.overrides] "gutter.assist"` sets it to anything, and a
//!    patched font's own lamp — U+F1A60, Nerd Fonts' `md-post_lamp` — is the
//!    value to reach for if you have one.
//! 2. **In a saved transcript: the words.** A file has no colour and no gutter, so
//!    [`export_line`] writes `Lanthorn: ` back onto the front of every assist line
//!    on its way out. That is the surface the original argument was really about
//!    — a pasted log, a bug report — and it still gets the words.
//! 3. **The kind.** [`TranscriptKind::Assist`] tags the line, so `/filter story`
//!    hides every assist and a player who wants 1982 gets 1982. Code can tell an
//!    assist from a slash dump without reading its text, which is why the export
//!    above can find the lines to mark in the first place.
//!
//! # The introduction, which is also a glyph self-test
//!
//! Once per session, above the first assist, [`preamble`] says:
//!
//! ```text
//! Lanthorn's Guiding Light: ● is mine, not the story's.
//! ```
//!
//! with the **configured** glyph substituted, never a hard-coded dot. That is
//! deliberate and load-bearing: a player whose font has no glyph for the mark
//! sees a tofu box in the one sentence that tells them what the box is. Reword
//! this line freely, but never into a sentence that omits
//! the icon.
//!
//! It is deliberately **just-in-time rather than a startup banner**. An earlier
//! draft announced the feature at session start — "is enabled and will display
//! guidance using ●" — which reads as a promise and costs a line of chrome on
//! every launch, including the many sessions where no assist ever fires. Firing
//! it above the first assist means it only ever appears when there is a mark on
//! screen to explain, and the sentence can therefore explain THAT MARK rather
//! than forecast one. It also opens with [`NAME`], which is what keeps
//! [`export_line`] from introducing it a second time in a saved file.
//!
//! # What an assist line looks like
//!
//! ```text
//! ● try instead — light · turn on · burn
//! ```
//!
//! — one glyph, in the gutter, drawn by `render::transcript`; the text itself is
//! exactly what the caller wrote.
//!
//! # A line may only claim what was actually established
//!
//! Those same three words come with two different openings, and which one is
//! used is not a matter of taste (SQ-1041, SQ-1121):
//!
//! * `this story knows — light` states a **fact about the dictionary**. It is
//!   verifiable from the story file, promises nothing about whether typing the
//!   word will do anything, and is therefore never wrong.
//! * `try instead — light` is a **recommendation**, and it is earned by
//!   [`crate::vocab`] having watched that command work in a silent copy of this
//!   game, from where the player is standing.
//!
//! Shipping the recommendation over an unvetted list would make the failure it
//! exists to fix strictly worse — confidently telling a player to try three
//! things that do not work is more frustrating than saying nothing at all. So
//! the wording follows the evidence: with the probe off, or unavailable, the
//! line drops back to the modest claim rather than overstating a weaker one.
//!
//! # What an assist line must never look like
//!
//! * **Bracketed.** `[…]` is the Z-machine parser's own voice. Never.
//! * **Hand-built.** [`AppState::push_assist`] is the only door; a line tagged
//!   [`TranscriptKind::Assist`] anywhere else has skipped the tone's style, the
//!   introduction and the export marker. `tests/suites/assist_voice.rs` fails any
//!   source file that tries.
//! * **In the story's second person.** "You could try turning on the lamp" is the
//!   game's voice — the story owns "you" (*"You are standing in an open
//!   field"*), and a helper borrowing it is the impersonation this register
//!   exists to prevent. Say what the *story* knows, or instruct in the plain
//!   imperative, and let the player decide: `this story knows — light · turn on
//!   · burn`, `try instead — light`. Note what the imperative buys besides the
//!   register: it reads at one suggestion as well as at four, where "try one of
//!   these" does not.
//! * **A boast, or an apology.** It fires mid-play, twenty times a session, and
//!   is sometimes wrong. Read every candidate line back on the twentieth firing
//!   and again assuming the suggestion is useless; anything that grates in either
//!   reading is the wrong line.
//! * **Long.** These arrive between the player's command and the game's reply, on
//!   a pane that may be forty columns wide. One line. Two if the second is a list.
//! * **A spoiler.** The assists volunteer; the hints wait to be asked. An assist
//!   may say hints exist and how to reach them, never what they say.
//!
//! # How to emit one
//!
//! ```ignore
//! state.push_assist(&Assist::help(format!("try instead — {}", verbs.join(" · "))));
//! state.push_assist(&Assist::caution("burning the leaflet cannot be undone."));
//! ```
//!
//! Build the [`Assist`] and hand it over whole: the text and its tone are one
//! subject and travel as one value, so a caller cannot supply the words and
//! forget the weight. Everything else — the kind, the style lookup, the
//! once-per-session introduction, and whether the player has the light switched
//! on at all — belongs to [`AppState::push_assist`].
//!
//! [`TranscriptKind::Meta`]: crate::state::TranscriptKind::Meta
//! [`TranscriptKind::Warning`]: crate::state::TranscriptKind::Warning
//! [`TranscriptKind::Assist`]: crate::state::TranscriptKind::Assist
//! [`AppState::push_assist`]: crate::state::AppState::push_assist

/// The name that marks a line as ours wherever words are the only carrier.
pub const NAME: &str = "Lanthorn";

/// What the feature is called, in the one sentence that introduces it.
pub const FEATURE: &str = "Lanthorn's Guiding Light";

/// The prefix an assist wears in a **saved transcript** and nowhere else.
///
/// On screen the mark in the gutter says this; a file has no gutter, so the words
/// go back on. Applied by [`export_line`] at the moment of export, which is why
/// the on-screen line and the exported one differ by exactly this string.
pub const EXPORT_PREFIX: &str = "Lanthorn: ";

/// Indent for an assist's continuation lines. Two spaces, which is what
/// `render::transcript`'s hanging wrap already treats as a continuation — and
/// what [`export_line`] reads to tell a continuation from a fresh assist.
pub const CONT_INDENT: &str = "  ";

/// The once-per-session introduction, shown above the first assist of a session.
///
/// Takes the glyph actually in force so the sentence shows the reader the mark
/// they will be seeing — and so a font missing it fails visibly HERE, in the one
/// line that also says what the mark means. Where to switch the feature off is
/// the ●/○ control in the pane border's own job. See the module docs.
pub fn preamble(icon: char) -> String {
    format!("{FEATURE}: {icon} is mine, not the story's.")
}

/// One transcript line as a **saved transcript** should carry it.
///
/// A file has no colour and no gutter glyph, so the assist's identity has to be
/// in its words there: this is the only place `Lanthorn: ` is ever applied. A
/// continuation (already indented by [`CONT_INDENT`]) hangs as it is, and a line
/// that already opens with the name — the introduction does — is left alone
/// rather than being introduced twice.
pub fn export_line(line: &str) -> String {
    if line.starts_with(CONT_INDENT) || line.starts_with(NAME) {
        line.to_string()
    } else {
        format!("{EXPORT_PREFIX}{line}")
    }
}

/// How much weight an assist carries. Two, deliberately: "here is something that
/// helps" and "you are about to do something you cannot undo". A third would be a
/// distinction the player cannot act on differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssistTone {
    /// The ordinary light: vocabulary, completions, where the hints are.
    Help,
    /// A consequence worth knowing before it happens (SQ-1043's irreversible
    /// move). Louder, because ignoring it costs the player their game.
    Caution,
}

impl AssistTone {
    /// The `style.toml` selector this tone draws in.
    ///
    /// Both resolve through the terminal's own **yellow** slot (the `alert` role)
    /// and differ by weight, the way `transcript_crash` differs from
    /// `transcript_warning`. A role rather than an RGB literal is what makes the
    /// light legible on a dark page and on Solarized Light's `#fdf6e3` alike,
    /// without lanthorn knowing which the player is on.
    pub fn selector(self) -> &'static str {
        match self {
            AssistTone::Help => "transcript_assist",
            AssistTone::Caution => "transcript_assist_caution",
        }
    }
}

/// One thing lanthorn has to say to the player, with the weight it carries.
///
/// The text and the tone are one subject, so they travel together rather than as
/// two arguments a caller can get out of order or supply half of (see CLAUDE.md's
/// refactoring policy). A later fact — which feature spoke, say, once the light
/// can be turned down per feature — is a field here, not another parameter at
/// every call site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assist {
    text: String,
    tone: AssistTone,
}

impl Assist {
    /// The ordinary light: vocabulary, completions, where the hints are.
    pub fn help(text: impl Into<String>) -> Self {
        Self { text: text.into(), tone: AssistTone::Help }
    }

    /// A consequence worth knowing before it happens.
    pub fn caution(text: impl Into<String>) -> Self {
        Self { text: text.into(), tone: AssistTone::Caution }
    }

    pub fn tone(&self) -> AssistTone {
        self.tone
    }

    /// The text, as the caller supplied it.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The transcript lines this assist becomes: the first as written — the mark
    /// in the gutter identifies it — and every continuation carrying
    /// [`CONT_INDENT`], so it hangs under the first instead of reading as prose
    /// (and so [`export_line`] can tell the two apart later).
    pub fn lines(&self) -> Vec<String> {
        self.text
            .split('\n')
            .enumerate()
            .map(|(i, l)| if i == 0 { l.to_string() } else { format!("{CONT_INDENT}{l}") })
            .collect()
    }
}

#[cfg(all(test, feature = "t-guidance"))]
mod tests {
    use super::*;

    #[test]
    fn the_first_line_is_the_callers_words_and_nothing_else() {
        assert_eq!(Assist::help("this story knows — light").lines(), vec!["this story knows — light"]);
        assert_eq!(Assist::caution("that cannot be undone.").lines(), vec!["that cannot be undone."]);
    }

    #[test]
    fn continuations_hang_rather_than_reading_as_story() {
        let a = Assist::help("this story knows:\nlight · turn on · burn");
        assert_eq!(a.lines(), vec!["this story knows:", "  light · turn on · burn"]);
    }

    #[test]
    fn the_two_tones_draw_from_different_selectors() {
        assert_ne!(AssistTone::Help.selector(), AssistTone::Caution.selector());
    }

    /// The screen's mark cannot travel in a file, so the words go back on there —
    /// once, and not onto a continuation or onto the introduction.
    #[test]
    fn an_exported_assist_carries_the_words_the_screen_did_not_need() {
        assert_eq!(export_line("this story knows — light"), "Lanthorn: this story knows — light");
        assert_eq!(export_line("  light · turn on · burn"), "  light · turn on · burn");
        assert_eq!(export_line(&preamble('●')), preamble('●'));
    }

    /// The register's own rule, applied to the constants: nothing lanthorn says
    /// here wears the Z-machine parser's brackets, and the introduction shows the
    /// glyph in force rather than a hard-coded one.
    #[test]
    fn the_introduction_names_the_feature_and_shows_the_mark_in_force() {
        let p = preamble('◈');
        assert!(!p.starts_with('['));
        assert!(p.starts_with(NAME), "the one line that identifies us says the name: {p:?}");
        assert!(p.contains('◈'), "the introduction must show the CONFIGURED mark: {p:?}");
        assert!(p.contains("story"), "and whose the marked lines are not: {p:?}");
        assert!(!EXPORT_PREFIX.starts_with('['));
    }
}
