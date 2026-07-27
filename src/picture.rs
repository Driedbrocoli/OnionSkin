//! Pictures to put on a page: a signature, a stamp, a logo.
//!
//! The commonest thing anybody adds to an already-printed document is their
//! own signature. Onionskin could put words and shapes on a page but not a
//! picture, which meant the one thing people most wanted to add was the one
//! thing it could not do.
//!
//! What comes out of here is a picture in the shape a PDF wants it, and the
//! choice between the two shapes is the whole of the module:
//!
//!   * A **JPEG** is carried through exactly as it arrived. PDF understands
//!     JPEG natively (`DCTDecode`), so re-encoding it would cost quality and
//!     size for nothing. A photograph of a letterhead stays 400 kB instead of
//!     becoming nine megabytes of raw samples.
//!
//!   * **Anything else** is decoded to plain samples and deflated. That is
//!     the only way to carry a PNG's transparency, which matters more than it
//!     sounds: a signature saved with a transparent background must not print
//!     as a signature inside a white rectangle, because the rectangle covers
//!     the line it is supposed to be sitting on.
//!
//! Transparency becomes a soft mask — a second, greyscale picture the same
//! size, where white means "show this" and black means "let the paper show".
//! It is how PDF has done transparency since 1.4 and every reader and printer
//! understands it.

use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum PictureError {
    #[error("no picture at {0}")]
    Missing(std::path::PathBuf),
    #[error(
        "{path} could not be read as a picture. Onionskin understands PNG, \
         JPEG, TIFF and BMP.\n    ({source})"
    )]
    Unreadable {
        path: std::path::PathBuf,
        source: image::ImageError,
    },
    #[error("{0} is an empty picture — it has no width or no height")]
    Empty(std::path::PathBuf),
}

/// A picture ready to be written into a PDF.
#[derive(Debug, Clone, PartialEq)]
pub enum Picture {
    /// JPEG bytes exactly as they arrived, for PDF to decode itself.
    Jpeg {
        bytes: Vec<u8>,
        width: u32,
        height: u32,
        /// Greyscale rather than colour, which PDF needs told separately.
        grey: bool,
    },
    /// Decoded samples: three bytes a pixel, and a byte of opacity each where
    /// the picture had any.
    Samples {
        width: u32,
        height: u32,
        rgb: Vec<u8>,
        /// One byte per pixel: 255 shows the picture, 0 shows the paper.
        alpha: Option<Vec<u8>>,
    },
}

impl Picture {
    pub fn width(&self) -> u32 {
        match self {
            Picture::Jpeg { width, .. } | Picture::Samples { width, .. } => *width,
        }
    }

    pub fn height(&self) -> u32 {
        match self {
            Picture::Jpeg { height, .. } | Picture::Samples { height, .. } => *height,
        }
    }

    /// How wide the picture is for every millimetre of height.
    ///
    /// The reason a caller can give one measurement and let the other follow:
    /// a signature squashed to a shape it was not is worse than no signature.
    pub fn aspect(&self) -> f64 {
        if self.height() == 0 {
            return 1.0;
        }
        self.width() as f64 / self.height() as f64
    }

    /// Whether any of it is see-through, which decides whether the PDF needs
    /// a soft mask alongside.
    pub fn has_transparency(&self) -> bool {
        matches!(self, Picture::Samples { alpha: Some(_), .. })
    }
}

/// Read a picture off disk, ready to be put on a page.
pub fn load(path: &Path) -> Result<Picture, PictureError> {
    if !path.is_file() {
        return Err(PictureError::Missing(path.to_path_buf()));
    }
    let bytes = std::fs::read(path).map_err(|source| PictureError::Unreadable {
        path: path.to_path_buf(),
        source: image::ImageError::IoError(source),
    })?;
    from_bytes(&bytes, path)
}

/// The same, from bytes already in hand.
pub fn from_bytes(bytes: &[u8], path: &Path) -> Result<Picture, PictureError> {
    let decoded = image::load_from_memory(bytes).map_err(|source| PictureError::Unreadable {
        path: path.to_path_buf(),
        source,
    })?;
    let (width, height) = (decoded.width(), decoded.height());
    if width == 0 || height == 0 {
        return Err(PictureError::Empty(path.to_path_buf()));
    }

    // A JPEG has no transparency to carry, and PDF reads JPEG itself — so the
    // best thing that can be done with the bytes is nothing at all.
    if is_jpeg(bytes) {
        let grey = matches!(
            decoded.color(),
            image::ColorType::L8 | image::ColorType::L16
        );
        return Ok(Picture::Jpeg {
            bytes: bytes.to_vec(),
            width,
            height,
            grey,
        });
    }

    let rgba = decoded.to_rgba8();
    let mut rgb = Vec::with_capacity((width as usize) * (height as usize) * 3);
    let mut alpha = Vec::with_capacity((width as usize) * (height as usize));
    let mut any_transparency = false;
    for pixel in rgba.pixels() {
        rgb.extend_from_slice(&pixel.0[..3]);
        alpha.push(pixel.0[3]);
        if pixel.0[3] != 255 {
            any_transparency = true;
        }
    }

    Ok(Picture::Samples {
        width,
        height,
        rgb,
        // A picture that is opaque everywhere gets no mask. Carrying one that
        // says "show all of it" is a second image in the file for nothing.
        alpha: if any_transparency { Some(alpha) } else { None },
    })
}

/// The two-byte mark every JPEG starts with.
fn is_jpeg(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xFF, 0xD8, 0xFF])
}

#[cfg(test)]
mod tests;
