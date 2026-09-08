// Z-machine interpreter error types.

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
}
