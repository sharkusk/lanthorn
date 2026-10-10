//! Status-line text shortening for embedding hosts that measure text in their
//! own units (SQ-1760).
//!
//! [`shorten_status_text`] applies ZMSD §8.2.2.2: "If the object's short name
//! exceeds the available room on the status line, the author suggests that an
//! interpreter should break it at the last space and append an ellipsis". The
//! host supplies the measure as a `fits` predicate, so a terminal counts
//! columns and a proportional-font host measures pixels; the rule itself is
//! decided once. The TUI's `truncate_status_text` is a thin column-counting
//! adapter over it.

/// Shorten `text` until it satisfies `fits`, the way ZMSD §8.2.2.2 asks.
///
/// We use the single-character ellipsis '…' rather than the spec's three dots,
/// and it is measured as part of `fits`, so a proportional host gets its width
/// right.
///
/// * `fits(text)` → `text` is returned unchanged (nothing gains a spurious '…').
/// * Otherwise the longest char-prefix `p` with `fits(p + "…")` is found, cut
///   back to its last space (trailing spaces trimmed), and `"…"` appended. A
///   `p` with no space (one long word) is kept whole: a mid-word cut, still
///   marked with the ellipsis.
/// * If not even `"…"` fits, the result is `""`.
///
/// `fits` is assumed monotone (a shorter string never fails where a longer one
/// fits). The cut is always on a char boundary.
pub fn shorten_status_text(text: &str, fits: impl Fn(&str) -> bool) -> String {
    if fits(text) {
        return text.to_string();
    }
    // Byte offsets of every char boundary, longest prefix first.
    let cuts: Vec<usize> = text
        .char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(text.len()))
        .collect();
    for &end in cuts.iter().rev() {
        let head = &text[..end];
        if !fits(&format!("{head}…")) {
            continue;
        }
        let kept = match head.rfind(' ') {
            Some(i) => head[..i].trim_end(),
            None => head,
        };
        return format!("{kept}…");
    }
    String::new()
}

#[cfg(all(test, feature = "t-render"))]
mod tests {
    use super::*;

    fn cols(w: usize) -> impl Fn(&str) -> bool {
        move |s| s.chars().count() <= w
    }

    #[test]
    fn fitting_text_is_unchanged() {
        assert_eq!(
            shorten_status_text("West of House", cols(13)),
            "West of House"
        );
    }

    #[test]
    fn breaks_at_the_last_space() {
        assert_eq!(shorten_status_text("West of House", cols(12)), "West of…");
        assert_eq!(shorten_status_text("Cyclops Room", cols(8)), "Cyclops…");
    }

    #[test]
    fn single_long_word_cuts_mid_word() {
        assert_eq!(shorten_status_text("Antechamber", cols(6)), "Antec…");
    }

    #[test]
    fn nothing_fits_gives_empty() {
        assert_eq!(shorten_status_text("Antechamber", cols(0)), "");
        assert_eq!(shorten_status_text("Antechamber", |_| false), "");
    }

    #[test]
    fn only_the_ellipsis_fits() {
        assert_eq!(shorten_status_text("Antechamber", cols(1)), "…");
    }

    #[test]
    fn ellipsis_width_counts_for_a_proportional_measure() {
        // 'W' is 3 units, '…' is 2, everything else 1.
        let width = |s: &str| {
            s.chars()
                .map(|c| match c {
                    'W' => 3,
                    '…' => 2,
                    _ => 1,
                })
                .sum::<usize>()
        };
        let fits = |s: &str| width(s) <= 9;
        // Prefix "West o" + '…' = 8+2 = 10 > 9; "West " + '…' = 7+2 = 9 fits,
        // and cutting back to the last space gives "West…".
        assert_eq!(shorten_status_text("West of House", fits), "West…");
        // The same text under a column count keeps "West of".
        assert_eq!(shorten_status_text("West of House", cols(9)), "West of…");
    }

    #[test]
    fn multibyte_chars_are_never_split() {
        assert_eq!(shorten_status_text("Café Noir", cols(9)), "Café Noir");
        assert_eq!(shorten_status_text("Café Noir", cols(8)), "Café…");
        assert_eq!(shorten_status_text("Éééééé", cols(4)), "Ééé…");
    }
}
