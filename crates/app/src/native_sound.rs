//! Sound effects a release disk carries natively, for stories that came off one
//! rather than out of a Blorb (SQ-0907).
//!
//! Two Infocom games use sound at all — *The Lurking Horror* and *Sherlock* — and on
//! the Amiga their disks hold a `Sound/` directory; the Macintosh compilation lays
//! the same sounds out as `/MAC/SOUND`. `blorb::infocom_sound` decodes either and
//! hands back AIFF, so what this module produces is indistinguishable downstream from
//! a Blorb's `Snd ` resource: same decoder, same volume scaling, same repeats, same
//! finish routine.
//!
//! **Effect numbers come from the `sN.nam` INDEX, never from a filename.** On *The
//! Lurking Horror* every index happens to name `sN.dat`, so the numbering looks like
//! a convention; on *Sherlock* the samples are called `armor`, `growl`, `splash`,
//! `violin.bin`, and effects 11, 12 and 13 all name the same `heart` sample at three
//! different pitches. A reader keyed on `sN.dat` finds nothing on that disk.
//!
//! The medium is read through [`crate::assets::files`], the door that already
//! enumerates whatever the story was mounted out of — so this is not an
//! Amiga-specific path, it is "whatever the disk has, laid out the way Infocom lays
//! it out". Read once at launch, because a sound has to start on the turn the game
//! asks for it.

use std::collections::HashMap;
use std::path::Path;

/// One sound the story's own medium carries.
#[derive(Debug, Clone)]
pub struct DiskSound {
    /// The Z-machine effect number, from the index that named this sample.
    pub effect: u16,
    /// The sample file's own name, as the index gives it — `s10.dat`, `growl`,
    /// `violin.bin`. Worth carrying because it is what a person recognises in the
    /// browser's info panel.
    pub name: String,
    /// Playback rate in Hz: the disk's own, bent by the effect's pitch file against
    /// the note the sample states it was recorded at, and capped at what Paula can
    /// clock. Nine of the twenty-nine effects across the two games are bent — see
    /// `blorb::infocom_sound::Pitch`, which is where the model and its evidence live.
    pub rate: u32,
    /// Sample count.
    pub frames: usize,
    /// The sample wrapped as AIFF, ready for the host's existing decoder.
    pub aiff: Vec<u8>,
}

/// Every sound on the story's own medium, by effect number.
///
/// Empty for a loose story and for a disk with no `Sound/` directory. Effects below
/// 3 are dropped: ZMSD §9 reserves 1 and 2 for the interpreter's own bleeps, which
/// are synthesised and never sampled.
pub fn from_medium(story_path: &Path) -> HashMap<u16, DiskSound> {
    // One pass over the medium, because reading it twice means mounting it twice.
    let files: Vec<(String, Vec<u8>)> = crate::assets::files(story_path)
        .into_iter()
        .filter(|f| f.is_on_medium())
        // Only the sound drawers are read: on a CD the rest is the disc.
        .filter(|f| blorb::infocom_sound::on_a_sound_path(&f.name))
        .filter_map(|f| {
            let name = f.name.clone();
            f.into_bytes().map(|b| (name, b))
        })
        .collect();
    blorb::infocom_sound::from_volume(files.iter().map(|(p, b)| (p.as_str(), b.as_slice())))
        .into_iter()
        .map(|(effect, (name, snd))| {
            (
                effect,
                DiskSound {
                    effect,
                    name,
                    rate: snd.effective_rate(),
                    frames: snd.samples.len(),
                    aiff: snd.to_aiff(),
                },
            )
        })
        .collect()
}
