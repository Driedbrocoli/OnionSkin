//! Showing a page of paper on the screen.
//!
//! Rendering a PDF page costs tens of milliseconds and uploading it to the
//! graphics card costs more, so a page that is drawn sixty times a second must
//! be rendered once and kept. What is kept is keyed on the file, the page and
//! the width asked for, because a window that is resized wants a sharper
//! picture and a window that is not wants the one it already has.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use eframe::egui;

use onionskin::geometry::PageSize;

/// One page, rendered and ready to draw.
pub struct Sheet {
    pub texture: egui::TextureHandle,
    /// The paper's real size, so a click can be turned into millimetres.
    pub page: PageSize,
}

#[derive(PartialEq, Eq, Hash)]
struct Key {
    file: PathBuf,
    page: usize,
    width_px: u32,
}

/// Every page rendered so far.
#[derive(Default)]
pub struct Previews {
    sheets: HashMap<Key, Sheet>,
    /// What went wrong last time, so a broken file is reported once rather
    /// than retried sixty times a second for as long as the window is open.
    failed: HashMap<PathBuf, String>,
}

impl Previews {
    /// The page, rendering it if it has not been seen at this size.
    ///
    /// Returns `Err` with something worth showing if the file cannot be drawn.
    pub fn sheet(
        &mut self,
        ctx: &egui::Context,
        file: &Path,
        page: usize,
        width_px: u32,
    ) -> Result<&Sheet, String> {
        // Rounded, so nudging the window by a pixel does not re-render.
        let width_px = (width_px.max(200) / 64) * 64;
        let key = Key {
            file: file.to_path_buf(),
            page,
            width_px,
        };

        if let Some(why) = self.failed.get(file) {
            return Err(why.clone());
        }
        if self.sheets.contains_key(&key) {
            return Ok(&self.sheets[&key]);
        }

        match render(ctx, file, page, width_px) {
            Ok(sheet) => {
                self.sheets.insert(key, sheet);
                let key = Key {
                    file: file.to_path_buf(),
                    page,
                    width_px,
                };
                Ok(&self.sheets[&key])
            }
            Err(why) => {
                self.failed.insert(file.to_path_buf(), why.clone());
                Err(why)
            }
        }
    }

    /// Forget everything about a file, after it has been written to.
    pub fn forget(&mut self, file: &Path) {
        self.sheets.retain(|key, _| key.file != file);
        self.failed.remove(file);
    }

    /// How many pages a document has, or `None` if it cannot be opened.
    pub fn page_count(&mut self, file: &Path) -> Option<usize> {
        let guard = onionskin::render::engine().ok()?;
        let document = guard.open(file).ok()?;
        Some(document.len())
    }
}

fn render(
    ctx: &egui::Context,
    file: &Path,
    page: usize,
    width_px: u32,
) -> Result<Sheet, String> {
    // An image is a page of paper too, and the commonest thing anybody has:
    // a scan. Drawing it needs no PDF renderer at all.
    if is_image(file) {
        let image = image::open(file).map_err(|e| format!("{e}"))?.to_rgb8();
        let (w, h) = (image.width(), image.height());
        let colour = egui::ColorImage::from_rgb([w as usize, h as usize], image.as_raw());
        return Ok(Sheet {
            texture: ctx.load_texture(
                format!("{}#{page}", file.display()),
                colour,
                egui::TextureOptions::LINEAR,
            ),
            // A bare image says nothing about the paper it came from. A4 is
            // the honest default and the screen that needs better asks the
            // person, rather than this guessing from the pixel count.
            page: PageSize {
                width_mm: 210.0,
                height_mm: 297.0,
            },
        });
    }

    let guard = onionskin::render::engine().map_err(|e| e.to_string())?;
    let document = guard.open(file).map_err(|e| e.to_string())?;
    if document.is_empty() {
        return Err("That document has no pages in it.".into());
    }
    let index = page.min(document.len() - 1);
    let size = document.page_sizes[index];

    // The resolution that gives the width asked for, so nothing is scaled
    // twice — once by the renderer and again by the graphics card.
    let dpi = (width_px as f64 / (size.width_mm / 25.4)).clamp(36.0, 400.0);
    let rendered = document.render(index, dpi).map_err(|e| e.to_string())?;

    let colour = egui::ColorImage::from_rgb(
        [rendered.width, rendered.height],
        &rendered.rgb,
    );
    Ok(Sheet {
        texture: ctx.load_texture(
            format!("{}#{index}@{width_px}", file.display()),
            colour,
            egui::TextureOptions::LINEAR,
        ),
        page: size,
    })
}

pub fn is_image(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "png" | "jpg" | "jpeg" | "tif" | "tiff" | "bmp"
    )
}
