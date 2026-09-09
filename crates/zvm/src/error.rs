//! Z-machine interpreter error types.

/// Everything that can be wrong with a story image, save file, screen
/// snapshot or paint log this crate is asked to read. [`crate::memory::Memory::new`]
/// returns this to name why a story image doesn't fit; a host surfaces the
/// variant to the player or developer rather than treating every failure as
/// the same generic error.
#[derive(Debug, PartialEq)]
#[non_exhaustive]
pub enum ZError {
    /// The story file is too short to contain a valid header (< 64 bytes).
    NotAStoryFile,
    /// Z-machine versions 1 and 2 are too old to be supported; other unknown
    /// versions are also rejected here.
    UnsupportedVersion(u8),
    /// A memory or data access fell outside the story file bounds.
    Truncated,
    /// The save file is for a different story (release/serial/checksum mismatch).
    SaveMismatch,
    /// A screen snapshot ([`crate::screen_snapshot`]) is not a snapshot at all,
    /// is truncated, or is otherwise unreadable. Unlike a save file this carries
    /// no story identity, so there is nothing here to mismatch — only to be
    /// malformed.
    BadScreenSnapshot,
    /// A screen snapshot was written by a NEWER format version than this build
    /// understands. Both numbers are named because the only useful thing a host
    /// can tell the player is which build wrote it and which is reading it.
    ScreenSnapshotVersion {
        /// The format version number the snapshot itself declares.
        found: u16,
        /// The highest format version this build knows how to read.
        supported: u16,
    },
    /// A paint log ([`crate::paint_log`]) is not a paint log at all, is
    /// truncated, or is otherwise unreadable. Carries no story identity, as
    /// [`Self::BadScreenSnapshot`] does not either.
    BadPaintLog,
    /// A paint log was written by a NEWER format version than this build
    /// understands.
    PaintLogVersion {
        /// The format version number the paint log itself declares.
        found: u16,
        /// The highest format version this build knows how to read.
        supported: u16,
    },
}
