//! Reading a zip, because a Word or OpenDocument file is one.
//!
//! [`crate::package`] writes zips; this reads them. The two halves are kept
//! apart because they have nothing in common but the format: writing is a
//! matter of choosing what to put in, and reading is a matter of surviving
//! whatever somebody else put in. A file arriving from another program is
//! damaged, truncated, or built by a tool with its own ideas about the spec
//! often enough that every field here is checked before it is believed.
//!
//! Only what a document needs is here: the central directory, stored and
//! deflated entries, and the zip64 fields for the sizes. Encryption, split
//! archives and the other compression methods are refused by name rather than
//! read wrongly.

/// The most a single entry may inflate to.
///
/// A `content.xml` is measured in hundreds of kilobytes. Anything claiming to
/// be a quarter of a gigabyte is either not a document or is a zip bomb — a
/// few kilobytes on disk that fill memory when read — and neither is worth
/// finding out about by running out of memory.
const MOST_ONE_ENTRY_MAY_HOLD: u64 = 256 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum ZipError {
    #[error("this is not a Word or OpenDocument file (it is not a zip archive)")]
    NotAZip,
    #[error("the file is damaged: {0}")]
    Damaged(String),
    #[error("{name} is compressed in a way Onionskin cannot read (method {method})")]
    Method { name: String, method: u16 },
    #[error("the file is password-protected. Open it, save an unprotected copy, and use that")]
    Encrypted,
}

/// An opened archive. Nothing is inflated until it is asked for, so a document
/// with fifty megabytes of photographs in it costs nothing to open.
pub struct Archive<'a> {
    bytes: &'a [u8],
    entries: Vec<Located>,
}

/// The names inside, and not the bytes — printing an archive should not print
/// the document.
impl std::fmt::Debug for Archive<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Archive")
            .field("bytes", &self.bytes.len())
            .field("entries", &self.entries)
            .finish()
    }
}

#[derive(Debug)]
struct Located {
    name: String,
    method: u16,
    encrypted: bool,
    packed: u64,
    size: u64,
    /// Where the local header sits.
    at: u64,
    crc: u32,
}

/// A little-endian integer at an offset, or `None` past the end.
fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn u64_at(bytes: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(bytes.get(at..at + 8)?.try_into().ok()?))
}

const END_OF_DIRECTORY: u32 = 0x0605_4b50;
const ZIP64_LOCATOR: u32 = 0x0706_4b50;
const ZIP64_END: u32 = 0x0606_4b50;
const DIRECTORY_ENTRY: u32 = 0x0201_4b50;
const LOCAL_HEADER: u32 = 0x0403_4b50;

impl<'a> Archive<'a> {
    pub fn open(bytes: &'a [u8]) -> Result<Archive<'a>, ZipError> {
        let end = find_end(bytes).ok_or(ZipError::NotAZip)?;

        let mut count = u16_at(bytes, end + 10).unwrap_or(0) as u64;
        let mut directory_size = u32_at(bytes, end + 12).unwrap_or(0) as u64;
        let mut directory_at = u32_at(bytes, end + 16).unwrap_or(0) as u64;

        // A zip64 archive writes 0xFFFF.. in the old fields and the real
        // numbers in a record of its own, found through a locator sitting
        // immediately before the end record.
        if count == 0xFFFF || directory_at == 0xFFFF_FFFF || directory_size == 0xFFFF_FFFF {
            if let Some(locator) = end.checked_sub(20) {
                if u32_at(bytes, locator) == Some(ZIP64_LOCATOR) {
                    let record = u64_at(bytes, locator + 8).unwrap_or(0) as usize;
                    if u32_at(bytes, record) == Some(ZIP64_END) {
                        count = u64_at(bytes, record + 32).unwrap_or(count);
                        directory_size = u64_at(bytes, record + 40).unwrap_or(directory_size);
                        directory_at = u64_at(bytes, record + 48).unwrap_or(directory_at);
                    }
                }
            }
        }

        // Where the directory says it is, and where it actually is, differ by a
        // constant when something has been stuck on the front of the archive —
        // which is how a self-extracting file is made, and also what happens
        // when a file is downloaded with a stray header. Every offset inside
        // then needs the same correction.
        let mut shift: i64 = 0;
        if u32_at(bytes, directory_at as usize) != Some(DIRECTORY_ENTRY) {
            let expected = (bytes.len() as i64) - 22 - (directory_size as i64);
            if expected >= 0 && u32_at(bytes, expected as usize) == Some(DIRECTORY_ENTRY) {
                shift = expected - directory_at as i64;
                directory_at = expected as u64;
            } else {
                return Err(ZipError::Damaged(
                    "the list of files inside it is missing".into(),
                ));
            }
        }

        let mut entries = Vec::new();
        let mut at = directory_at as usize;
        // `count` comes out of the file and cannot be trusted to be small, so
        // the walk is bounded by the bytes rather than by the count.
        for _ in 0..count.min(1_000_000) {
            if u32_at(bytes, at) != Some(DIRECTORY_ENTRY) {
                break;
            }
            let flags = u16_at(bytes, at + 8).unwrap_or(0);
            let method = u16_at(bytes, at + 10).unwrap_or(0);
            let crc = u32_at(bytes, at + 16).unwrap_or(0);
            let mut packed = u32_at(bytes, at + 20).unwrap_or(0) as u64;
            let mut size = u32_at(bytes, at + 24).unwrap_or(0) as u64;
            let name_len = u16_at(bytes, at + 28).unwrap_or(0) as usize;
            let extra_len = u16_at(bytes, at + 30).unwrap_or(0) as usize;
            let comment_len = u16_at(bytes, at + 32).unwrap_or(0) as usize;
            let mut offset = u32_at(bytes, at + 42).unwrap_or(0) as u64;

            let name_at = at + 46;
            let Some(raw) = bytes.get(name_at..name_at + name_len) else {
                break;
            };
            // Names are UTF-8 when bit 11 says so and code page 437 otherwise.
            // Every producer of these documents writes UTF-8 whatever the flag,
            // and the names in question are all ASCII, so reading them as UTF-8
            // and replacing anything invalid is right and cannot mislead.
            let name = String::from_utf8_lossy(raw).into_owned();

            let extra_at = name_at + name_len;
            if let Some(extra) = bytes.get(extra_at..extra_at + extra_len) {
                read_zip64_extra(extra, &mut size, &mut packed, &mut offset);
            }

            entries.push(Located {
                name,
                method,
                // Bits 0 and 6 are the two kinds of encryption; either means
                // the bytes are not the file.
                encrypted: flags & 0x0001 != 0 || flags & 0x0040 != 0,
                packed,
                size,
                at: (offset as i64 + shift).max(0) as u64,
                crc,
            });
            at = extra_at + extra_len + comment_len;
        }

        if entries.is_empty() {
            return Err(ZipError::Damaged("it contains no files".into()));
        }
        Ok(Archive { bytes, entries })
    }

    /// Every file in the archive, in the order the directory lists them.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|e| e.name.as_str())
    }

    pub fn has(&self, name: &str) -> bool {
        self.entries.iter().any(|e| e.name == name)
    }

    /// One file's contents, inflated.
    pub fn read(&self, name: &str) -> Result<Vec<u8>, ZipError> {
        let entry = self
            .entries
            .iter()
            .find(|e| e.name == name)
            .ok_or_else(|| ZipError::Damaged(format!("{name} is missing from it")))?;

        if entry.encrypted {
            return Err(ZipError::Encrypted);
        }

        // The local header repeats the name and may carry a different amount of
        // extra data than the central one, so where the bytes begin can only be
        // read from here.
        let header = entry.at as usize;
        if u32_at(self.bytes, header) != Some(LOCAL_HEADER) {
            return Err(ZipError::Damaged(format!("{name} is not where it says")));
        }
        let name_len = u16_at(self.bytes, header + 26).unwrap_or(0) as usize;
        let extra_len = u16_at(self.bytes, header + 28).unwrap_or(0) as usize;
        let from = header + 30 + name_len + extra_len;
        let packed = self
            .bytes
            .get(from..from + entry.packed as usize)
            .ok_or_else(|| ZipError::Damaged(format!("{name} is cut short")))?;

        let out = match entry.method {
            0 => packed.to_vec(),
            8 => inflate(packed, entry.size, name)?,
            other => {
                return Err(ZipError::Method {
                    name: name.to_string(),
                    method: other,
                })
            }
        };

        // The checksum is the only thing that says the bytes came out as they
        // went in. A document that fails it is damaged, and reading half a
        // paragraph out of it would be worse than saying so.
        if entry.crc != 0 && crate::package::crc32(&out) != entry.crc {
            return Err(ZipError::Damaged(format!(
                "{name} does not match its checksum"
            )));
        }
        Ok(out)
    }

    /// The first of these that is present, with its name.
    pub fn read_any(&self, names: &[&str]) -> Option<(String, Vec<u8>)> {
        for name in names {
            if self.has(name) {
                if let Ok(bytes) = self.read(name) {
                    return Some((name.to_string(), bytes));
                }
            }
        }
        None
    }
}

/// The end-of-archive record, which is the only fixed point in a zip.
///
/// It is at the very end unless the archive has a comment, and a comment may be
/// up to 64 kB — so the last 64 kB and a bit are searched backwards, and the
/// first plausible record wins.
fn find_end(bytes: &[u8]) -> Option<usize> {
    if bytes.len() < 22 {
        return None;
    }
    let earliest = bytes.len().saturating_sub(22 + 0xFFFF);
    for at in (earliest..=bytes.len() - 22).rev() {
        if u32_at(bytes, at) == Some(END_OF_DIRECTORY) {
            let comment = u16_at(bytes, at + 20).unwrap_or(0) as usize;
            // The record has to account for every byte after it, or it is not
            // the record — it is those four bytes appearing inside a file.
            if at + 22 + comment == bytes.len() {
                return Some(at);
            }
        }
    }
    None
}

/// The zip64 extended information field, which replaces whichever of the sizes
/// were too large to write in thirty-two bits. They appear in a fixed order,
/// and only the ones that overflowed are present.
fn read_zip64_extra(extra: &[u8], size: &mut u64, packed: &mut u64, offset: &mut u64) {
    let mut at = 0usize;
    while at + 4 <= extra.len() {
        let id = u16_at(extra, at).unwrap_or(0);
        let len = u16_at(extra, at + 2).unwrap_or(0) as usize;
        let body = at + 4;
        if id == 0x0001 {
            let mut take = body;
            let mut next = |value: &mut u64| {
                if *value == 0xFFFF_FFFF && take + 8 <= body + len {
                    if let Some(read) = u64_at(extra, take) {
                        *value = read;
                    }
                    take += 8;
                }
            };
            next(size);
            next(packed);
            next(offset);
            return;
        }
        at = body + len;
    }
}

/// Deflate, undone.
fn inflate(packed: &[u8], expected: u64, name: &str) -> Result<Vec<u8>, ZipError> {
    use std::io::Read;

    let ceiling = expected.min(MOST_ONE_ENTRY_MAY_HOLD);
    let mut out = Vec::with_capacity(ceiling.min(1 << 20) as usize);
    // One byte past the ceiling, so an entry that lies about its size is
    // caught below rather than silently truncated.
    let mut reader = flate2::read::DeflateDecoder::new(packed).take(ceiling + 1);
    reader
        .read_to_end(&mut out)
        .map_err(|e| ZipError::Damaged(format!("{name} could not be uncompressed: {e}")))?;

    if out.len() as u64 > ceiling {
        return Err(ZipError::Damaged(format!(
            "{name} is far larger inside than it claims"
        )));
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
