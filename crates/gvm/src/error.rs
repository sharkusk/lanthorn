//! Glulx interpreter load/parse error types.
//!
//! These are *load-time* errors returned by `Memory::new`. Runtime faults (bad
//! opcode, out-of-range access, div-by-zero) never use these — they record a
//! diagnostic string and Quit, per the Phase 2a design.

/// Errors that can arise while loading a Glulx image.
#[derive(Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum GError {
    /// The image is too short to contain the 36-byte header.
    TooShort,
    /// The first four bytes are not the `"Glul"` magic.
    BadMagic,
    /// The image's major version is outside the supported range (2 or 3).
    UnsupportedVersion(u32),
    /// RAMSTART/EXTSTART/ENDMEM are inconsistent or not 256-byte aligned.
    BadMemoryMap,
    /// The image is shorter than EXTSTART (missing stored initial memory).
    Truncated,
    /// Save-state data (Glulx-Quetzal) is corrupt, truncated, or inconsistent.
    /// Returned by [`crate::exec::Machine::restore_state`]; never a panic.
    BadSave(String),
    /// A header field demands more resource than the interpreter will allocate
    /// (ENDMEM or the requested stack size beyond the sanity cap): the story
    /// is rejected at load rather than attempting a multi-gigabyte
    /// allocation from a 36-byte file. (SQ-0624)
    LimitExceeded(&'static str),
}
