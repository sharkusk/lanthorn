//! Picture-resource lookup — the seam behind ZMSD §15 `picture_data`, SQ-1402.
//!
//! `picture_data(picture-number, table)` [branch] answers one of two
//! questions depending on `picture-number`: number `0` asks "how many
//! pictures does this file hold, and what is the file's own release
//! number" (written to `table` as two words: count, then release), branching
//! if any pictures are available; any other number asks "what is this
//! picture's height and width" (written as height then width), branching if
//! that number exists and leaving `table` untouched — and not branching —
//! if it doesn't.
//!
//! Before this module, the only way to answer that opcode was
//! [`crate::cpu::exec::Machine::set_picture_dims`]: a pre-filled
//! `Vec<(u16, u16, u16)>` the host had to build by enumerating every
//! picture in its archive before the story ever ran. [`Resources`] is the
//! same three questions asked as a trait instead of a table, so a host whose
//! archive is streamed, lazily decoded, or simply expensive to enumerate up
//! front can answer on demand. [`PictureTable`] is the eager, vector-backed
//! answer every host used before — `Machine::set_picture_dims` and
//! [`crate::cpu::boot::BootConfig::with_picture_dims`] both build one, so
//! neither host behaviour nor call site needs to change.
//!
//! Widths and heights are answered in the resource's OWN pixels, exactly the
//! unit `with_picture_dims`'s table was always built in — `BootConfig` is
//! what turns those into the Version 6 unit-screen pixels `picture_data`
//! actually reports (SQ-0479/SQ-0790), wrapping whichever [`Resources`] a
//! host installs the same way whether it is a [`PictureTable`] or a
//! caller's own implementation. See [`crate::cpu::boot::BootConfig::with_resources`].

/// Answers ZMSD §15 `picture_data`'s three questions about a picture-resource
/// file: how many pictures it holds, what release/version number the file
/// itself carries, and — for one specific picture — its width and height.
///
/// Implement this to answer on demand instead of pre-filling a
/// [`PictureTable`]; install one with
/// [`crate::cpu::exec::Machine::set_resources`] or
/// [`crate::cpu::boot::BootConfig::with_resources`].
pub trait Resources {
    /// Number of pictures available — `picture_data(0, …)` word 0. Zero means
    /// no pictures, which is also what makes `picture_data(0, …)` not branch.
    fn picture_count(&self) -> u16;
    /// The picture file's own release/version number — `picture_data(0, …)`
    /// word 1. Purely informational to the story; ZMSD does not constrain it
    /// further.
    fn picture_release(&self) -> u16;
    /// Width and height of `number`, in this resource's own pixels, or
    /// `None` if the number is unknown — the difference between
    /// `picture_data(number, …)` branching true (and writing `table`) or not
    /// branching at all.
    fn picture_dims(&self, number: u16) -> Option<(u16, u16)>;
}

/// A host that never calls [`crate::cpu::exec::Machine::set_resources`] or
/// [`crate::cpu::boot::BootConfig::with_resources`]/`with_picture_dims` sees
/// this: no pictures, so `picture_data(0, …)` reports zero and does not
/// branch, and every specific number answers `None`.
#[derive(Debug, Default)]
pub(crate) struct EmptyResources;

impl Resources for EmptyResources {
    fn picture_count(&self) -> u16 {
        0
    }
    fn picture_release(&self) -> u16 {
        0
    }
    fn picture_dims(&self, _number: u16) -> Option<(u16, u16)> {
        None
    }
}

/// The eager, vector-backed [`Resources`] every host answered `picture_data`
/// with before this trait existed: a pre-filled `(number, width, height)`
/// table plus the release number `picture_data(0, …)` reports.
///
/// [`crate::cpu::exec::Machine::set_picture_dims`] and
/// [`crate::cpu::boot::BootConfig::with_picture_dims`] both build one of
/// these from a plain `Vec<(u16, u16, u16)>`, so a host that already
/// enumerates its whole archive up front needs no code change.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PictureTable {
    dims: Vec<(u16, u16, u16)>,
    release: u16,
}

impl PictureTable {
    /// `dims` is `(picture_number, width_px, height_px)` triples; `release`
    /// is what `picture_data(0, …)` reports as the picture file's own
    /// release/version number.
    pub fn new(dims: Vec<(u16, u16, u16)>, release: u16) -> PictureTable {
        PictureTable { dims, release }
    }
}

impl Resources for PictureTable {
    fn picture_count(&self) -> u16 {
        self.dims.len() as u16
    }
    fn picture_release(&self) -> u16 {
        self.release
    }
    fn picture_dims(&self, number: u16) -> Option<(u16, u16)> {
        self.dims.iter().find(|&&(n, _, _)| n == number).map(|&(_, w, h)| (w, h))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picture_table_answers_count_release_and_dims() {
        let t = PictureTable::new(vec![(5, 100, 60), (9, 20, 30)], 42);
        assert_eq!(t.picture_count(), 2);
        assert_eq!(t.picture_release(), 42);
        assert_eq!(t.picture_dims(5), Some((100, 60)));
        assert_eq!(t.picture_dims(9), Some((20, 30)));
        assert_eq!(t.picture_dims(1), None, "unknown picture number");
    }

    #[test]
    fn empty_resources_answers_nothing() {
        let e = EmptyResources;
        assert_eq!(e.picture_count(), 0);
        assert_eq!(e.picture_release(), 0);
        assert_eq!(e.picture_dims(0), None);
    }
}
