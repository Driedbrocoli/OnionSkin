use super::*;

/// A small picture written out in a real format, the way somebody's signature
/// would arrive.
fn written(format: image::ImageFormat, colour: image::ColorType) -> Vec<u8> {
    let mut buffer = std::io::Cursor::new(Vec::new());
    match colour {
        image::ColorType::Rgba8 => {
            let mut img = image::RgbaImage::new(4, 2);
            // Half of it see-through, which is the whole point of a signature.
            for (x, _y, pixel) in img.enumerate_pixels_mut() {
                *pixel = if x < 2 {
                    image::Rgba([10, 20, 30, 255])
                } else {
                    image::Rgba([0, 0, 0, 0])
                };
            }
            image::DynamicImage::ImageRgba8(img)
        }
        image::ColorType::L8 => image::DynamicImage::ImageLuma8(image::GrayImage::new(4, 2)),
        _ => {
            let mut img = image::RgbImage::new(4, 2);
            for pixel in img.pixels_mut() {
                *pixel = image::Rgb([10, 20, 30]);
            }
            image::DynamicImage::ImageRgb8(img)
        }
    }
    .write_to(&mut buffer, format)
    .expect("should have written");
    buffer.into_inner()
}

#[test]
fn a_jpeg_is_carried_through_exactly_as_it_arrived() {
    // PDF reads JPEG itself, so re-encoding would cost quality and size for
    // nothing. A photographed letterhead stays the size it was.
    let bytes = written(image::ImageFormat::Jpeg, image::ColorType::Rgb8);
    let picture = from_bytes(&bytes, Path::new("logo.jpg")).unwrap();
    match &picture {
        Picture::Jpeg {
            bytes: kept,
            width,
            height,
            grey,
        } => {
            assert_eq!(kept, &bytes, "the JPEG was not carried through untouched");
            assert_eq!((*width, *height), (4, 2));
            assert!(!grey);
        }
        other => panic!("a JPEG came back as {other:?}"),
    }
    assert!(!picture.has_transparency());
}

#[test]
fn a_greyscale_jpeg_says_so_because_pdf_has_to_be_told() {
    let bytes = written(image::ImageFormat::Jpeg, image::ColorType::L8);
    match from_bytes(&bytes, Path::new("stamp.jpg")).unwrap() {
        Picture::Jpeg { grey, .. } => assert!(grey),
        other => panic!("came back as {other:?}"),
    }
}

#[test]
fn a_transparent_png_keeps_its_transparency() {
    // The reason this matters: a signature saved with a see-through
    // background must not print inside a white box, because the box covers
    // the line it is meant to be sitting on.
    let bytes = written(image::ImageFormat::Png, image::ColorType::Rgba8);
    let picture = from_bytes(&bytes, Path::new("signature.png")).unwrap();
    assert!(picture.has_transparency(), "{picture:?}");
    match picture {
        Picture::Samples {
            width,
            height,
            rgb,
            alpha,
        } => {
            assert_eq!((width, height), (4, 2));
            assert_eq!(rgb.len(), 4 * 2 * 3);
            let alpha = alpha.expect("checked above");
            assert_eq!(alpha.len(), 4 * 2);
            // Solid on the left, see-through on the right.
            assert_eq!(alpha[0], 255);
            assert_eq!(alpha[3], 0);
        }
        other => panic!("came back as {other:?}"),
    }
}

#[test]
fn a_picture_that_is_solid_all_over_carries_no_mask() {
    // A mask saying "show all of it" is a second picture in the file for
    // nothing.
    let bytes = written(image::ImageFormat::Png, image::ColorType::Rgb8);
    let picture = from_bytes(&bytes, Path::new("logo.png")).unwrap();
    assert!(!picture.has_transparency(), "{picture:?}");
    match picture {
        Picture::Samples { alpha, rgb, .. } => {
            assert!(alpha.is_none());
            assert_eq!(&rgb[..3], &[10, 20, 30]);
        }
        other => panic!("came back as {other:?}"),
    }
}

#[test]
fn the_shape_of_the_picture_comes_back_so_one_measurement_is_enough() {
    // Give a width and let the height follow, so a signature is never
    // squashed into a shape it was not.
    let bytes = written(image::ImageFormat::Png, image::ColorType::Rgb8);
    let picture = from_bytes(&bytes, Path::new("logo.png")).unwrap();
    assert_eq!(picture.width(), 4);
    assert_eq!(picture.height(), 2);
    assert!((picture.aspect() - 2.0).abs() < 1e-9);
}

#[test]
fn a_file_that_is_not_a_picture_says_so_by_name() {
    let said = from_bytes(b"this is just text", Path::new("notes.txt"))
        .unwrap_err()
        .to_string();
    assert!(said.contains("notes.txt"), "{said}");
    assert!(said.contains("PNG"), "{said}");
}

#[test]
fn a_missing_file_is_missing_rather_than_unreadable() {
    let dir = tempfile::tempdir().unwrap();
    assert!(matches!(
        load(&dir.path().join("not-there.png")),
        Err(PictureError::Missing(_))
    ));
}

#[test]
fn a_real_file_on_disk_loads_the_same_as_its_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("signature.png");
    let bytes = written(image::ImageFormat::Png, image::ColorType::Rgba8);
    std::fs::write(&path, &bytes).unwrap();
    assert_eq!(load(&path).unwrap(), from_bytes(&bytes, &path).unwrap());
}
