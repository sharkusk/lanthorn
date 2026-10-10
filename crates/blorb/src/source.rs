//! Where a volume reader's bytes come from (SQ-1761).
//!
//! Every filesystem reader in this crate used to own its image as a `Vec<u8>`.
//! That is the right shape for a floppy — 800 KB, read once, indexed freely —
//! and the wrong one for a CD: opening one game off the 354 MB *Masterpieces*
//! disc meant reading all of it (and then copying 308 MB of it out of its sector
//! frames), over whatever the disc happened to be mounted on.
//!
//! [`Source`] is the seam. The readers ask it for ranges — `get(at..end)` — and
//! it neither knows nor cares what filesystem is asking:
//!
//! * [`Source::Mem`] is today's behaviour, byte for byte: the image in memory,
//!   a range is a borrowed slice.
//! * [`Source::Disc`] is a window onto a CD image that stays on disk. A range is
//!   a seek and a read of just those sectors, mapped through the same
//!   [`Sectors`] logic the in-memory path uses (cooked: logical byte *n* is file
//!   byte *n*; raw: 2048 bytes of user data per `stride`-byte frame), so the two
//!   paths cannot disagree about where a byte is.
//!
//! A [`Disc`] is one open file and can carry several windows — the Macintosh
//! partition of a hybrid disc is one, the whole medium another — which is why
//! the windows hold it by `Arc`.

use crate::cd::{Sectors, USER_DATA};
use std::borrow::Cow;
use std::cell::Cell;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::ops::Range;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// How much of the front of a disc is read to measure its framing and to sniff
/// its volume descriptors. ISO 9660's primary volume descriptor is logical
/// sector 16: 32,768 bytes cooked, and ends at 39,696 in a raw 2352-byte frame.
pub(crate) const HEAD: usize = 48 * 1024;

/// The most sectors one physical read gathers from a raw dump (about 600 KB).
const RAW_CHUNK_SECTORS: usize = 256;

thread_local! {
    static BYTES_READ: Cell<u64> = const { Cell::new(0) };
}

/// Add `n` to this thread's count of bytes read from image files.
pub(crate) fn count_bytes(n: usize) {
    BYTES_READ.with(|c| c.set(c.get() + n as u64));
}

/// How many bytes of image files this thread has read from disk — whole-file
/// reads and sector reads alike. For a test to assert that a CD launch touched
/// a few megabytes of a 354 MB file.
pub fn bytes_read_on_this_thread() -> u64 {
    BYTES_READ.with(Cell::get)
}

/// A CD image left on disk, read by sector.
#[derive(Debug)]
pub(crate) struct Disc {
    file: Mutex<File>,
    sectors: Sectors,
    /// User-data bytes the image holds; a trailing partial frame is not one.
    logical_len: usize,
    /// The last sector a small read touched. A header check reads 64 bytes and
    /// the next reads 512 from the same place; the second should cost nothing.
    last: Mutex<Option<(usize, Arc<Vec<u8>>)>>,
}

impl Disc {
    /// Open `path` and measure its framing from its first [`HEAD`] bytes.
    pub(crate) fn open(path: &Path) -> std::io::Result<Disc> {
        let mut file = File::open(path)?;
        let file_len = file.metadata()?.len() as usize;
        let mut head = vec![0u8; HEAD.min(file_len)];
        file.read_exact(&mut head)?;
        count_bytes(head.len());
        let sectors = Sectors::of(&head);
        Ok(Disc {
            file: Mutex::new(file),
            sectors,
            logical_len: sectors.logical_len_of(file_len),
            last: Mutex::new(None),
        })
    }

    /// How the file's bytes relate to logical sectors.
    pub(crate) fn sectors(&self) -> Sectors {
        self.sectors
    }

    /// User-data bytes the image holds.
    pub(crate) fn logical_len(&self) -> usize {
        self.logical_len
    }

    fn read_physical(&self, at: u64, len: usize) -> Option<Vec<u8>> {
        let mut file = self.file.lock().ok()?;
        file.seek(SeekFrom::Start(at)).ok()?;
        let mut buf = vec![0u8; len];
        file.read_exact(&mut buf).ok()?;
        count_bytes(len);
        Some(buf)
    }

    /// Exactly `len` logical bytes from logical offset `at`, or `None` when they
    /// are not all there.
    pub(crate) fn read(&self, at: usize, len: usize) -> Option<Vec<u8>> {
        if at.checked_add(len)? > self.logical_len {
            return None;
        }
        if len == 0 {
            return Some(Vec::new());
        }
        if at / USER_DATA == (at + len - 1) / USER_DATA {
            let sector = self.sector(at / USER_DATA)?;
            let off = at % USER_DATA;
            return sector.get(off..off + len).map(<[u8]>::to_vec);
        }
        match self.sectors {
            Sectors::Cooked => self.read_physical(at as u64, len),
            Sectors::Raw { stride, data } => {
                let mut out = Vec::with_capacity(len);
                let (mut sector, mut off) = (at / USER_DATA, at % USER_DATA);
                while out.len() < len {
                    let want = len - out.len();
                    let count = (off + want).div_ceil(USER_DATA).min(RAW_CHUNK_SECTORS);
                    // One read spanning `count` frames, from the first one's user
                    // data to the last one's.
                    let from = sector * stride + data;
                    let span = (count - 1) * stride + USER_DATA;
                    let buf = self.read_physical(from as u64, span)?;
                    for i in 0..count {
                        let skip = if i == 0 { off } else { 0 };
                        let take = (USER_DATA - skip).min(len - out.len());
                        let start = i * stride + skip;
                        out.extend_from_slice(&buf[start..start + take]);
                    }
                    sector += count;
                    off = 0;
                }
                Some(out)
            }
        }
    }

    /// One sector's user data (short only for a cooked image's last), through the
    /// one-entry cache.
    fn sector(&self, n: usize) -> Option<Arc<Vec<u8>>> {
        if let Some((cached, bytes)) = self.last.lock().ok()?.as_ref() {
            if *cached == n {
                return Some(Arc::clone(bytes));
            }
        }
        let len = USER_DATA.min(self.logical_len.checked_sub(n * USER_DATA)?);
        let from = match self.sectors {
            Sectors::Cooked => n * USER_DATA,
            Sectors::Raw { stride, data } => n * stride + data,
        };
        let bytes = Arc::new(self.read_physical(from as u64, len)?);
        *self.last.lock().ok()? = Some((n, Arc::clone(&bytes)));
        Some(bytes)
    }

    /// Up to `len` logical bytes from `at`, short at the end of the image — the
    /// in-memory [`Sectors::copy`]'s contract.
    pub(crate) fn copy(&self, at: usize, len: usize) -> Vec<u8> {
        let len = len.min(self.logical_len.saturating_sub(at));
        self.read(at, len).unwrap_or_default()
    }
}

/// The bytes a volume reader reads: in memory, or a window onto a disc.
#[derive(Debug, Clone)]
pub(crate) enum Source {
    /// The whole image, held.
    Mem(Arc<Vec<u8>>),
    /// `len` logical bytes of a disc starting at logical byte `base`.
    Disc { disc: Arc<Disc>, base: usize, len: usize },
}

impl Source {
    /// A window onto `disc`.
    pub(crate) fn window(disc: &Arc<Disc>, base: usize, len: usize) -> Source {
        Source::Disc { disc: Arc::clone(disc), base, len }
    }

    /// How many bytes the source holds.
    pub(crate) fn len(&self) -> usize {
        match self {
            Source::Mem(v) => v.len(),
            Source::Disc { len, .. } => *len,
        }
    }

    /// The bytes in `range`, or `None` unless all of them are there.
    pub(crate) fn get(&self, range: Range<usize>) -> Option<Cow<'_, [u8]>> {
        match self {
            Source::Mem(v) => v.get(range).map(Cow::Borrowed),
            Source::Disc { disc, base, len } => {
                if range.end > *len || range.start > range.end {
                    return None;
                }
                disc.read(base + range.start, range.end - range.start).map(Cow::Owned)
            }
        }
    }

    /// Up to `len` bytes from `at`, short at the end — never more than the
    /// source holds.
    pub(crate) fn get_clamped(&self, at: usize, len: usize) -> Cow<'_, [u8]> {
        let end = at.saturating_add(len).min(self.len());
        self.get(at.min(end)..end).unwrap_or_default()
    }
}
