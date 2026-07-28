//! Tests for the local web UI.

use super::*;
use std::path::Path;

/// Build a multipart body the way a browser would.
fn multipart(fields: &[(&str, Option<&str>, &[u8])]) -> (String, Vec<u8>) {
    let boundary = "----OnionskinTestBoundary";
    let mut body = Vec::new();
    for (name, filename, data) in fields {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        match filename {
            Some(file) => body.extend_from_slice(
                format!(
                    "Content-Disposition: form-data; name=\"{name}\"; filename=\"{file}\"\r\n\
                     Content-Type: application/octet-stream\r\n\r\n"
                )
                .as_bytes(),
            ),
            None => body.extend_from_slice(
                format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes(),
            ),
        }
        body.extend_from_slice(data);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

// ---------------------------------------------------------------------------
// Reading a form
// ---------------------------------------------------------------------------

#[test]
fn a_form_comes_apart_into_its_fields() {
    let (content_type, body) = multipart(&[
        ("mode", None, b"vector"),
        ("original", Some("report.pdf"), b"%PDF-1.4 pretend"),
    ]);
    let parts = parse_multipart(&content_type, &body).unwrap();

    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0].name, "mode");
    assert_eq!(parts[0].text(), "vector");
    assert_eq!(parts[1].name, "original");
    assert_eq!(parts[1].filename.as_deref(), Some("report.pdf"));
    assert_eq!(parts[1].data, b"%PDF-1.4 pretend");
}

#[test]
fn binary_content_survives_intact() {
    // A PDF is bytes, not text: anything that goes through a string on the way
    // in will mangle it.
    let bytes: Vec<u8> = (0u8..=255).cycle().take(5000).collect();
    let (content_type, body) = multipart(&[("original", Some("a.pdf"), &bytes)]);

    let parts = parse_multipart(&content_type, &body).unwrap();
    assert_eq!(parts[0].data, bytes);
}

#[test]
fn a_field_that_is_empty_stays_empty() {
    let (content_type, body) = multipart(&[("profile", None, b"")]);
    let parts = parse_multipart(&content_type, &body).unwrap();
    assert_eq!(parts.len(), 1);
    assert!(parts[0].data.is_empty());
}

#[test]
fn a_form_with_no_boundary_is_refused_rather_than_guessed_at() {
    let err = parse_multipart("multipart/form-data", b"whatever").unwrap_err();
    assert!(err.contains("where one field ends"), "{err}");
}

#[test]
fn a_body_that_is_not_a_form_yields_nothing_rather_than_panicking() {
    for rubbish in [
        &b"this is not a form at all"[..],
        &b""[..],
        &b"--xyz"[..],
        &b"--xyz\r\nno headers here"[..],
    ] {
        let parts = parse_multipart("multipart/form-data; boundary=xyz", rubbish).unwrap();
        assert!(parts.is_empty(), "{:?}", String::from_utf8_lossy(rubbish));
    }
}

// ---------------------------------------------------------------------------
// File names from a browser
// ---------------------------------------------------------------------------

#[test]
fn an_uploaded_name_cannot_reach_out_of_the_workspace() {
    // A browser sends whatever the file was called, and a name is a perfectly
    // ordinary string right up until it is joined onto a path.
    for hostile in [
        "../../.ssh/authorized_keys",
        "..\\..\\windows\\system32\\evil.dll",
        "/etc/passwd",
        "....//....//x",
        "..",
    ] {
        let safe = safe_name(hostile, "upload.pdf");
        assert!(!safe.contains(".."), "{hostile} -> {safe}");
        assert!(!safe.contains('/'), "{hostile} -> {safe}");
        assert!(!safe.contains('\\'), "{hostile} -> {safe}");
        assert!(!safe.starts_with('.'), "{hostile} -> {safe}");
    }
}

#[test]
fn an_ordinary_name_is_left_alone() {
    assert_eq!(
        safe_name("Purchase Order 4471.pdf", "x"),
        "Purchase Order 4471.pdf"
    );
    assert_eq!(safe_name("report-v2.docx", "x"), "report-v2.docx");
}

#[test]
fn a_name_with_nothing_usable_in_it_gets_the_fallback() {
    assert_eq!(safe_name("", "upload.pdf"), "upload.pdf");
    assert_eq!(safe_name("///", "upload.pdf"), "upload.pdf");
    assert_eq!(safe_name("……", "upload.pdf"), "upload.pdf");
}

// ---------------------------------------------------------------------------
// The page itself
// ---------------------------------------------------------------------------

#[test]
fn the_page_fetches_nothing_from_anywhere() {
    // The promise is that nothing of yours leaves this machine. A page that pulls a
    // stylesheet from someone else's server breaks that the moment it is
    // opened, silently, and no test of the Rust would notice.
    let lower = page().to_lowercase();
    for forbidden in [
        "http://", "https://", "//cdn", "<script", "src=\"//", "@import", "url(http",
    ] {
        assert!(
            !lower.contains(forbidden),
            "the page contains {forbidden:?}"
        );
    }
}

#[test]
fn the_page_says_how_to_print_it() {
    // The commonest way to waste a sheet is "Fit to page".
    assert!(page().contains("100%"));
    assert!(page().contains("Fit to page"));
    assert!(page().contains("never sends your documents anywhere"));
}

#[test]
fn the_results_page_fetches_nothing_from_anywhere_either() {
    // The same promise, and the same way of breaking it. This page is built
    // rather than written out whole, so it needs its own check.
    let page = result_page(
        &["note: something worth knowing".into()],
        "abc123",
        std::time::Duration::from_secs(3),
    );
    let lower = page.to_lowercase();
    for forbidden in [
        "http://", "https://", "//cdn", "<script", "src=\"//", "@import", "url(http",
    ] {
        assert!(
            !lower.contains(forbidden),
            "the page contains {forbidden:?}"
        );
    }
}

#[test]
fn the_results_page_says_what_happened_and_offers_the_delta() {
    let page = result_page(
        &[
            "note: Onionskin read the document itself, without a word processor.\n    \
             The lines may not break where Word breaks them."
                .into(),
            "WARNING [page 1]: ink lands within 3 mm of the edge".into(),
        ],
        "tok123",
        std::time::Duration::from_secs(90),
    );
    assert!(page.contains("read the document itself"), "{page}");
    assert!(page.contains("may not break where Word"), "{page}");
    assert!(page.contains("ink lands within 3 mm"), "{page}");
    // The warning is set apart from the note, or the two read alike.
    assert!(page.contains("class=\"warn\""), "{page}");
    assert!(page.contains("class=\"note\""), "{page}");
    assert!(page.contains("href=\"/delta/tok123\""), "{page}");
    assert!(page.contains("Fit to page"), "{page}");
}

#[test]
fn a_run_worth_waiting_for_says_how_long_it_took() {
    // A browser shows nothing at all while it waits, so afterwards is the only
    // chance to say that the waiting was the work rather than a fault.
    let quick = result_page(&["note: x".into()], "t", std::time::Duration::from_secs(8));
    assert!(quick.contains("Took 8 seconds"), "{quick}");

    let slow = result_page(
        &["note: x".into()],
        "t",
        std::time::Duration::from_secs(180),
    );
    assert!(slow.contains("Took 3 minutes"), "{slow}");

    // And says nothing at all when there was nothing to wait for.
    let instant = result_page(
        &["note: x".into()],
        "t",
        std::time::Duration::from_millis(80),
    );
    assert!(!instant.contains("Took"), "{instant}");
}

#[test]
fn a_document_named_with_a_bracket_cannot_close_a_tag() {
    let page = result_page(
        &["note: <b>Smith & Sons</b> was opened".into()],
        "t",
        std::time::Duration::from_millis(200),
    );
    assert!(page.contains("&lt;b&gt;Smith &amp; Sons"), "{page}");
    assert!(!page.contains("<b>Smith"), "{page}");
}

#[test]
fn a_delta_set_aside_is_collected_once_and_then_gone() {
    let token = set_aside(b"%PDF-1.4 pretend".to_vec());
    assert_eq!(collect(&token).as_deref(), Some(&b"%PDF-1.4 pretend"[..]));
    assert!(
        collect(&token).is_none(),
        "collecting twice should not hand out the same delta again"
    );
}

#[test]
fn only_the_last_few_deltas_are_kept() {
    // A person who makes deltas and never collects them should not fill the
    // machine's memory with documents.
    let first = set_aside(vec![1u8; 16]);
    for _ in 0..MOST_WAITING {
        set_aside(vec![2u8; 16]);
    }
    assert!(
        collect(&first).is_none(),
        "the oldest should have been dropped"
    );
}

#[test]
fn the_page_asks_for_both_documents() {
    assert!(page().contains("name=\"original\""));
    assert!(page().contains("name=\"edited\""));
    assert!(page().contains("enctype=\"multipart/form-data\""));
    assert!(page().contains("action=\"/delta\""));
}

// ---------------------------------------------------------------------------
// Serving
// ---------------------------------------------------------------------------

/// Start a server on a free port and hand back its address.
fn start() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);

    let host = address.ip().to_string();
    let port = address.port();
    std::thread::spawn(move || {
        let _ = serve(&host, port);
    });
    // Wait for it to come up rather than guessing at a sleep.
    for _ in 0..200 {
        if TcpStream::connect(address).is_ok() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    address.to_string()
}

/// The barest HTTP client, so the tests need no dependency either.
fn request(address: &str, raw: &[u8]) -> String {
    let mut stream = TcpStream::connect(address).unwrap();
    stream.write_all(raw).unwrap();
    stream.flush().unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    String::from_utf8_lossy(&response).to_string()
}

fn get(address: &str, path: &str) -> String {
    request(
        address,
        format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").as_bytes(),
    )
}

/// Post, saying what the caller will take back.
fn post_accepting(address: &str, accept: &str, content_type: &str, body: &[u8]) -> String {
    let mut raw = format!(
        "POST /delta HTTP/1.1\r\nHost: localhost\r\nAccept: {accept}\r\n\
         Content-Type: {content_type}\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    raw.extend_from_slice(body);
    request(address, &raw)
}

fn post(address: &str, content_type: &str, body: &[u8]) -> String {
    let mut raw = format!(
        "POST /delta HTTP/1.1\r\nHost: localhost\r\nContent-Type: {content_type}\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    raw.extend_from_slice(body);
    request(address, &raw)
}

#[test]
fn the_server_serves_its_one_page() {
    let address = start();
    let response = get(&address, "/");

    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response:.80}");
    assert!(response.contains("text/html"));
    assert!(response.contains("Onionskin"));
    assert!(
        response.contains("Content-Security-Policy"),
        "no CSP header"
    );
}

#[test]
fn a_page_that_is_not_there_says_where_the_one_page_is() {
    let address = start();
    let response = get(&address, "/nowhere");
    assert!(response.starts_with("HTTP/1.1 404"));
    assert!(response.contains("one page"));
}

#[test]
fn the_server_says_it_is_alive() {
    let address = start();
    assert!(get(&address, "/health").contains("ok"));
}

#[test]
fn a_request_with_no_documents_asks_for_them_rather_than_failing_obscurely() {
    let address = start();
    let (content_type, body) = multipart(&[("mode", None, b"raster")]);
    let response = post(&address, &content_type, &body);

    assert!(response.starts_with("HTTP/1.1 422"), "{response}");
    assert!(response.contains("Choose both files"), "{response}");
}

#[test]
fn an_upload_past_the_limit_is_refused_before_it_is_read() {
    let address = start();
    let raw = format!(
        "POST /delta HTTP/1.1\r\nHost: localhost\r\n\
         Content-Type: multipart/form-data; boundary=x\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n",
        MAX_BODY + 1
    );
    let response = request(&address, raw.as_bytes());
    assert!(response.starts_with("HTTP/1.1 400"), "{response}");
    assert!(response.contains("limit is"), "{response}");
}

/// Two one-page PDFs, the second with an extra line at `y_mm`.
fn a_pair(dir: &Path, second_line: Option<(&str, f64)>) -> (Vec<u8>, Vec<u8>) {
    let page = crate::geometry::PageSize::new(210.0, 297.0);
    let line = |text: &str, y: f64| crate::pdf::PlacedLine {
        text: text.into(),
        x_mm: 25.0,
        y_mm: y,
        size_pt: 14.0,
        font: crate::pdf::LineFont::Builtin(crate::pdf::Font::Helvetica),
        rotation_deg: 0.0,
        colour: (0.0, 0.0, 0.0),
    };
    let before = dir.join("before.pdf");
    let after = dir.join("after.pdf");
    crate::pdf::write_delta(&before, &[page], &[vec![line("Report", 40.0)]], "t", None).unwrap();

    let mut lines = vec![line("Report", 40.0)];
    if let Some((text, y)) = second_line {
        lines.push(line(text, y));
    }
    crate::pdf::write_delta(&after, &[page], &[lines], "t", None).unwrap();
    (
        std::fs::read(&before).unwrap(),
        std::fs::read(&after).unwrap(),
    )
}

#[test]
fn two_documents_come_back_as_a_delta() {
    let Ok(_) = crate::render::engine() else {
        return;
    };
    let address = start();
    let dir = tempfile::tempdir().unwrap();
    let (original, edited) = a_pair(dir.path(), Some(("Approved", 150.0)));

    let (content_type, body) = multipart(&[
        ("original", Some("before.pdf"), &original),
        ("edited", Some("after.pdf"), &edited),
        ("dpi", None, b"150"),
    ]);
    let response = post(&address, &content_type, &body);

    // A run nearly always has something worth saying — at the very least that
    // no calibration profile was used — so what comes back first is the page
    // that says it, with the delta offered underneath.
    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response:.300}");
    assert!(response.contains("text/html"), "{response:.300}");
    assert!(response.contains("The delta is ready"), "{response:.400}");

    let link = response
        .split("href=\"/delta/")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .expect("the page should offer the delta");

    let collected = get(&address, &format!("/delta/{link}"));
    assert!(collected.contains("application/pdf"), "{collected:.300}");
    assert!(collected.contains("filename=\"delta.pdf\""));
    assert!(collected.contains("%PDF"), "the body is not a PDF");

    // And only once: the file is handed over, not stored.
    let again = get(&address, &format!("/delta/{link}"));
    assert!(again.starts_with("HTTP/1.1 404"), "{again:.200}");
}

#[test]
fn a_caller_that_asks_for_the_file_gets_the_file() {
    // Everything worth saying about a run is worth saying to a person, and a
    // browser is a person. A script posted two documents to get a delta, and
    // handing it a page of prose instead would break it.
    let Ok(_) = crate::render::engine() else {
        return;
    };
    let address = start();
    let dir = tempfile::tempdir().unwrap();
    let (original, edited) = a_pair(dir.path(), Some(("Approved", 150.0)));

    let (content_type, body) = multipart(&[
        ("original", Some("before.pdf"), &original),
        ("edited", Some("after.pdf"), &edited),
        ("dpi", None, b"150"),
    ]);
    let response = post_accepting(&address, "application/pdf", &content_type, &body);

    assert!(response.contains("application/pdf"), "{response:.300}");
    assert!(response.contains("filename=\"delta.pdf\""));
    assert!(response.contains("%PDF"), "the body is not a PDF");
    assert!(
        !response.contains("The delta is ready"),
        "it should be the file, not the page about it"
    );
}

#[test]
fn two_identical_documents_are_refused_with_the_reason() {
    let Ok(_) = crate::render::engine() else {
        return;
    };
    let address = start();
    let dir = tempfile::tempdir().unwrap();
    let (original, edited) = a_pair(dir.path(), None);

    let (content_type, body) = multipart(&[
        ("original", Some("before.pdf"), &original),
        ("edited", Some("after.pdf"), &edited),
        ("dpi", None, b"150"),
    ]);
    let response = post(&address, &content_type, &body);

    assert!(response.starts_with("HTTP/1.1 422"), "{response:.300}");
    assert!(response.contains("Not safe to print"), "{response}");
    assert!(response.contains("edited file second"), "{response}");
}

// ---------------------------------------------------------------------------
// Turning a scan into something editable
// ---------------------------------------------------------------------------

/// Post to any path, so the conversion endpoint can be reached as well as the
/// delta one.
fn post_to(address: &str, path: &str, content_type: &str, body: &[u8]) -> Vec<u8> {
    let mut raw = format!(
        "POST {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: {content_type}\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    raw.extend_from_slice(body);

    let mut stream = TcpStream::connect(address).unwrap();
    stream.write_all(&raw).unwrap();
    stream.flush().unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    response
}

/// Split a response into its headers and its body, which for a Word file is
/// not text and cannot be handled as a string.
fn split_response(response: &[u8]) -> (String, Vec<u8>) {
    let at = response
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .unwrap_or(response.len());
    (
        String::from_utf8_lossy(&response[..at]).to_string(),
        response.get(at + 4..).unwrap_or(&[]).to_vec(),
    )
}

/// A scanned sheet with a line of writing on it, drawn from a real font.
fn a_scan_and_its_font() -> Option<(Vec<u8>, Vec<u8>)> {
    let font_path = Path::new("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf");
    let font_bytes = std::fs::read(font_path).ok()?;
    let font = crate::font::EmbeddedFont::load(font_path).ok()?;

    // A4 at 200 dpi, with one line of type on it.
    let page = crate::geometry::PageSize {
        width_mm: 210.0,
        height_mm: 297.0,
    };
    let dpi = 200.0;
    let width = (page.width_mm / 25.4 * dpi).round() as u32;
    let height = (page.height_mm / 25.4 * dpi).round() as u32;
    let mut image = image::GrayImage::from_pixel(width, height, image::Luma([245u8]));

    let size_pt = 14.0;
    let em_mm = size_pt * 25.4 / 72.0;
    let upem = font.units_per_em();
    let per_mm = dpi / 25.4;
    let mut pen = 25.0f64;
    let text = "INVOICE 4471";
    let widths: Vec<f64> = font
        .shape(text)
        .ok()?
        .iter()
        .map(|g| g.advance_1000 / 1000.0)
        .collect();

    for (index, ch) in text.chars().enumerate() {
        if let Some(contours) = font.outline(ch) {
            // Fill each contour by testing points against it, which is slow and
            // perfectly adequate for one short line.
            let placed: Vec<Vec<(f64, f64)>> = contours
                .iter()
                .map(|c| {
                    c.iter()
                        .map(|&(gx, gy)| (pen + gx / upem * em_mm, 40.0 - gy / upem * em_mm))
                        .collect()
                })
                .collect();
            let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
            for contour in &placed {
                for &(x, y) in contour {
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x);
                    y1 = y1.max(y);
                }
            }
            for py in (y0 * per_mm) as u32..=((y1 * per_mm).ceil() as u32).min(height - 1) {
                let y = (py as f64 + 0.5) / per_mm;
                let mut crossings: Vec<f64> = Vec::new();
                for contour in &placed {
                    for i in 0..contour.len() {
                        let (ax, ay) = contour[i];
                        let (bx, by) = contour[(i + 1) % contour.len()];
                        if (ay > y) == (by > y) {
                            continue;
                        }
                        crossings.push(ax + (y - ay) / (by - ay) * (bx - ax));
                    }
                }
                crossings.sort_by(|a, b| a.partial_cmp(b).unwrap());
                for span in crossings.chunks_exact(2) {
                    let from = (span[0] * per_mm).round().max(0.0) as u32;
                    let to = ((span[1] * per_mm).round() as u32).min(width - 1);
                    for px in from..=to {
                        image.put_pixel(px, py, image::Luma([25u8]));
                    }
                }
            }
        }
        pen += widths.get(index).copied().unwrap_or(0.5) * em_mm;
    }

    let mut png = Vec::new();
    image::DynamicImage::ImageLuma8(image)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .ok()?;
    Some((png, font_bytes))
}

#[test]
fn the_page_offers_to_read_a_scan() {
    // The HTML is wrapped for reading, so a sentence can straddle a line. What
    // matters is that it is said, not where it breaks.
    let flowed = page();
    let flowed = flowed.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(page().contains("action=\"/convert\""), "no conversion form");
    // The font is still offered, for an alphabet the built-in faces do not
    // cover — but as an option, and the page has to say so. Presenting it as
    // required is what made people go looking for a .ttf they did not need.
    assert!(flowed.contains("The font the page was set in"), "{flowed}");
    assert!(
        flowed.contains("optional"),
        "the page does not say the font is optional:\n{flowed}"
    );
    assert!(
        flowed.contains("works out which face"),
        "the page does not say Onionskin will work the face out itself"
    );
    for offered in ["docx", "odt", "onionskin"] {
        assert!(page().contains(offered), "{offered} is not offered");
    }
}

#[test]
fn a_scan_comes_back_as_a_word_document() {
    let Some((scan, font)) = a_scan_and_its_font() else {
        eprintln!("no DejaVu on this machine; skipping");
        return;
    };
    let address = start();
    let (content_type, body) = multipart(&[
        ("scan", Some("page.png"), &scan),
        ("font", Some("DejaVuSans.ttf"), &font),
        ("format", None, b"docx"),
        ("page", None, b"a4"),
    ]);
    let (headers, file) = split_response(&post_to(&address, "/convert", &content_type, &body));

    assert!(headers.starts_with("HTTP/1.1 200"), "{headers}");
    // Labelled as a Word document. A browser handed a .docx labelled as a PDF
    // offers to open it in a PDF viewer, and the person concludes it is broken.
    assert!(
        headers.contains("wordprocessingml.document"),
        "wrong content type:\n{headers}"
    );
    assert!(headers.contains("page.docx"), "{headers}");
    assert_eq!(&file[0..2], b"PK", "that is not a zip, so not a .docx");
    assert!(file.len() > 500, "only {} bytes came back", file.len());
}

/// A scan with no font is read anyway, against the faces on this machine.
///
/// This used to be a refusal that sent somebody to C:\\Windows\\Fonts. The
/// command line stopped asking that question when it learned to work out which
/// face a page is set in, and the browser — where the person is least likely
/// to know the answer, and least able to find a .ttf — went on asking it.
#[test]
fn a_scan_with_no_font_is_read_against_the_faces_on_this_machine() {
    let Some((scan, _)) = a_scan_and_its_font() else {
        eprintln!("no DejaVu on this machine; skipping");
        return;
    };
    let address = start();
    let (content_type, body) =
        multipart(&[("scan", Some("page.png"), &scan), ("format", None, b"docx")]);
    let (headers, file) = split_response(&post_to(&address, "/convert", &content_type, &body));

    assert!(
        headers.starts_with("HTTP/1.1 200"),
        "a scan without a font was refused:\n{headers}\n{}",
        String::from_utf8_lossy(&file)
    );
    assert_eq!(&file[0..2], b"PK", "that is not a zip, so not a .docx");
}

#[test]
fn something_that_is_not_an_image_is_refused_rather_than_crashing() {
    let address = start();
    let (content_type, body) = multipart(&[
        ("scan", Some("notes.txt"), b"this is not a scan"),
        ("font", Some("font.ttf"), b"nor is this a font"),
    ]);
    let (headers, message) = split_response(&post_to(&address, "/convert", &content_type, &body));
    assert!(headers.starts_with("HTTP/1.1 422"), "{headers}");
    assert!(
        String::from_utf8_lossy(&message).contains("image"),
        "{}",
        String::from_utf8_lossy(&message)
    );
}

#[test]
fn the_box_colours_the_form_offers_are_all_understood() {
    // Every value in the select has to map to something, or somebody choosing
    // blue gets red and no explanation.
    let page = page();
    for name in ["red", "blue", "green", "orange", "magenta", "black"] {
        assert!(
            page.contains(&format!("value=\"{name}\"")),
            "the form does not offer {name}"
        );
    }
    // Compared by hand rather than with assert_ne!, which on a tuple of f64
    // reads as a negated ordering and clippy is right to dislike it.
    let differ = |a: (f64, f64, f64), b: (f64, f64, f64)| {
        (a.0 - b.0).abs() > 1e-9 || (a.1 - b.1).abs() > 1e-9 || (a.2 - b.2).abs() > 1e-9
    };
    assert!(differ(outline_colour("blue"), outline_colour("red")));
    assert!(differ(outline_colour("green"), outline_colour("black")));
    // Anything else falls back rather than refusing: a form can be posted by
    // hand with any value in it, and that is not worth losing a delta over.
    assert_eq!(outline_colour("chartreuse"), outline_colour("red"));
    assert_eq!(outline_colour(""), outline_colour("red"));
    assert_eq!(outline_colour("  BLUE  "), outline_colour("blue"));
}

#[test]
fn the_page_offers_the_box_round_every_change() {
    let page = page();
    assert!(page.contains("name=\"outline\""), "no outline checkbox");
    assert!(page.contains("name=\"outline_colour\""), "no colour choice");
    // And says what it costs, because the box goes onto the paper.
    assert!(page.contains("printed onto the paper"), "{page}");
}

// ---------------------------------------------------------------------------
// Reading a scan, in the browser
// ---------------------------------------------------------------------------

/// A request as the server would have built it from a browser's POST.
fn posted(fields: &[(&str, Option<&str>, &[u8])]) -> Request {
    let (content_type, body) = multipart(fields);
    Request {
        method: "POST".to_string(),
        path: "/convert".to_string(),
        content_type,
        body,
        accept: String::new(),
    }
}

/// The font was required, so the browser sent people hunting through
/// C:\Windows\Fonts for a question the page can answer itself — a question the
/// command line stopped asking a long time ago.
#[test]
fn the_font_is_no_longer_demanded_before_anything_is_read() {
    let said = convert_scan(&posted(&[
        ("scan", Some("scan.png"), b"not really a png"),
        ("format", None, b"docx"),
    ]))
    .unwrap_err();
    // It gets as far as looking at the scan, rather than stopping at the font.
    assert!(
        !said.contains("Choose the font"),
        "a missing font still stops the run before the scan is looked at: {said}"
    );
    assert!(
        said.contains("does not look like a scan"),
        "the refusal was not about the thing actually wrong: {said}"
    );
}

/// With no scan there is nothing to do, and the message has to name every kind
/// that would work — a PDF first, because that is what a scanner produces.
#[test]
fn a_missing_scan_names_the_kinds_that_would_work() {
    let said = convert_scan(&posted(&[("format", None, b"docx")])).unwrap_err();
    assert!(said.contains("PDF"), "{said}");
    assert!(said.contains("PNG"), "{said}");
}

/// A PDF is recognised by what is in it rather than by what it is called: a
/// browser sends whatever name the file had, and a scanner is as likely to
/// produce "Scan_001" with no extension at all.
#[test]
fn a_pdf_is_recognised_by_its_contents_not_its_name() {
    // Not a real PDF beyond the marker, so this fails in the renderer rather
    // than in the image decoder — which is the whole point: it was routed to
    // the renderer at all.
    let said = convert_scan(&posted(&[
        ("scan", Some("Scan_001"), b"%PDF-1.4 but not a whole one"),
        ("format", None, b"docx"),
    ]))
    .unwrap_err();
    assert!(
        !said.contains("does not look like a scan"),
        "a PDF with no extension was sent to the image decoder: {said}"
    );
}

/// The form must not ask for a font as though it were required, or the change
/// above is invisible to the person it was made for.
#[test]
fn the_form_asks_for_a_font_as_an_option_not_a_requirement() {
    let html = page();
    let font_input = html
        .lines()
        .find(|line| line.contains("id=\"font\""))
        .expect("a font input on the page");
    assert!(
        !font_input.contains("required"),
        "the font is still marked required: {font_input}"
    );
    assert!(
        html.contains("optional"),
        "nothing on the page says the font is optional"
    );
}

/// And it must offer to take a PDF, since that is what comes off a scanner.
#[test]
fn the_form_takes_a_pdf_as_well_as_a_picture() {
    let html = page();
    let scan_input = html
        .lines()
        .find(|line| line.contains("id=\"scan\""))
        .expect("a scan input on the page");
    assert!(
        scan_input.contains(".pdf"),
        "the file browser will not offer PDFs: {scan_input}"
    );
}

// ---------------------------------------------------------------------------
// Several PDFs, one after another
// ---------------------------------------------------------------------------

/// A one-page PDF, made the way the rest of the program makes one.
fn a_page(words: &str) -> Vec<u8> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("page.pdf");
    let sizes = [crate::geometry::PageSize::new(210.0, 297.0)];
    let lines = vec![vec![crate::pdf::PlacedLine {
        text: words.to_string(),
        x_mm: 20.0,
        y_mm: 40.0,
        size_pt: 12.0,
        font: crate::pdf::LineFont::Builtin(crate::pdf::Font::Helvetica),
        colour: (0.0, 0.0, 0.0),
        rotation_deg: 0.0,
    }]];
    crate::pdf::write_delta(&path, &sizes, &lines, "test", None).unwrap();
    std::fs::read(&path).unwrap()
}

/// The whole point: files in, one document out, in the order they were sent.
#[test]
fn several_pdfs_come_back_as_one_document_in_the_order_they_were_given() {
    let request = posted(&[
        ("files", Some("page-1.pdf"), &a_page("First")),
        ("files", Some("page-2.pdf"), &a_page("Second")),
        ("files", Some("page-3.pdf"), &a_page("Third")),
    ]);
    let (bytes, name) = join_files(&request).expect("three PDFs should join");
    assert_eq!(name, "joined.pdf");
    assert!(bytes.starts_with(b"%PDF"), "that is not a PDF");

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("joined.pdf");
    std::fs::write(&path, &bytes).unwrap();
    let read = lopdf::Document::load(&path).unwrap();
    let pages: Vec<u32> = read.get_pages().into_keys().collect();
    assert_eq!(pages.len(), 3);
    for (index, wanted) in ["First", "Second", "Third"].iter().enumerate() {
        let text = read.extract_text(&[pages[index]]).unwrap();
        assert!(
            text.contains(wanted),
            "page {} does not say {wanted}: {text:?}",
            index + 1
        );
    }
}

/// One file is not a join, and the message says what to do rather than what
/// went wrong.
#[test]
fn one_file_is_refused_with_something_to_do_about_it() {
    let request = posted(&[("files", Some("only.pdf"), &a_page("Only"))]);
    let said = join_files(&request).unwrap_err();
    assert!(said.contains("at least two"), "{said}");
    assert!(said.contains("several at once"), "{said}");

    // And nothing at all is the same answer, not a panic.
    assert!(join_files(&posted(&[])).is_err());
}

/// A browser sends whatever the file was called, and what it was called may be
/// `../../etc/passwd`. Only the last part of it is kept, and only the parts of
/// that which are plainly a name.
#[test]
fn a_filename_that_tries_to_climb_out_of_the_folder_does_not() {
    for hostile in [
        "../../../../etc/passwd",
        "..\\..\\windows\\system32\\config",
        "/etc/shadow",
        "....//....//escaped.pdf",
    ] {
        let kept = sanitised(hostile);
        if let Some(kept) = kept {
            assert!(!kept.contains('/'), "{hostile} kept a slash: {kept}");
            assert!(!kept.contains('\\'), "{hostile} kept a backslash: {kept}");
            assert!(
                !kept.starts_with('.'),
                "{hostile} kept a leading dot: {kept}"
            );
        }
    }
    // An ordinary name comes through unharmed, because mangling those would
    // make the report unreadable. A browser that sends a folder with it — some
    // do, for a whole directory — keeps the file's own name and not a run-on of
    // the path, which is the difference the split makes and the filter cannot.
    assert_eq!(sanitised("page-1.pdf").as_deref(), Some("page-1.pdf"));
    assert_eq!(sanitised("scans/page-1.pdf").as_deref(), Some("page-1.pdf"));
    assert_eq!(
        sanitised("C:\\Users\\me\\page-1.pdf").as_deref(),
        Some("page-1.pdf")
    );
    assert_eq!(
        sanitised("2024_invoice.PDF").as_deref(),
        Some("2024_invoice.PDF")
    );
    // And a name with nothing usable left in it is no name at all.
    assert_eq!(sanitised(""), None);
    assert_eq!(sanitised("..."), None);
    assert_eq!(sanitised("/"), None);
}

/// A hostile name must not stop the join working — the file is still a file,
/// whatever the browser called it.
#[test]
fn a_file_with_a_hostile_name_is_still_joined() {
    let request = posted(&[
        (
            "files",
            Some("../../../../tmp/escaped.pdf"),
            &a_page("First"),
        ),
        ("files", Some("page-2.pdf"), &a_page("Second")),
    ]);
    let (bytes, _) = join_files(&request).expect("it should still join");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("joined.pdf");
    std::fs::write(&path, &bytes).unwrap();
    assert_eq!(lopdf::Document::load(&path).unwrap().get_pages().len(), 2);
}

/// The page has to offer it, or the route is unreachable from the one place
/// this program serves.
#[test]
fn the_page_offers_the_join_and_says_what_it_does_not_do() {
    assert!(
        PAGE_BODY.contains("action=\"/join\""),
        "the page has no join form"
    );
    assert!(
        PAGE_BODY.contains("multiple"),
        "the picker will not take more than one file"
    );
    // And it is honest that this page is not the whole program, with somewhere
    // to go for the rest.
    assert!(
        PAGE_BODY.contains("onionskin-desktop"),
        "the window is not mentioned"
    );
    assert!(
        PAGE_BODY.contains("onionskin --help"),
        "the command line is not mentioned"
    );
}

/// The word really goes on, and it goes on grey.
///
/// The whole design of the watermark rests on the toner being light enough to
/// read the page through, and a browser that quietly sent full black would
/// destroy every sheet it was pointed at.
#[test]
fn the_browser_stamps_a_word_across_the_sheet_in_grey() {
    let request = posted(&[
        ("sheet", Some("report.pdf"), &a_page("Quarterly report")),
        ("text", None, b"DRAFT"),
    ]);
    let (bytes, name) = watermark_sheet(&request).expect("it should stamp");
    assert_eq!(name, "watermark.pdf");

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mark.pdf");
    std::fs::write(&path, &bytes).unwrap();

    let engine = crate::render::engine().expect("a renderer");
    let doc = engine.open(&path).expect("the delta should open");
    let drawn = doc.render_gray(0, 100.0).expect("it should draw");
    let ink: Vec<u8> = drawn
        .gray
        .iter()
        .copied()
        .filter(|level| *level < 250)
        .collect();
    assert!(!ink.is_empty(), "the delta came out blank");
    assert!(
        ink.iter().copied().min().unwrap_or(255) > 150,
        "the browser stamped something nearly black, which would bury the page"
    );
}

/// A grey typed as a percentage is taken as one. The form offers 0–100 because
/// that is how people say it, and 0.75 is what the placement wants.
#[test]
fn a_grey_typed_as_a_percentage_is_understood() {
    for (typed, darker) in [("20", true), ("0.2", true), ("75", false)] {
        let request = posted(&[
            ("sheet", Some("report.pdf"), &a_page("Quarterly report")),
            ("text", None, b"VOID"),
            ("grey", None, typed.as_bytes()),
        ]);
        let (bytes, _) = watermark_sheet(&request).expect("it should stamp");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mark.pdf");
        std::fs::write(&path, &bytes).unwrap();
        let engine = crate::render::engine().expect("a renderer");
        let doc = engine.open(&path).expect("it should open");
        let drawn = doc.render_gray(0, 100.0).expect("it should draw");
        let darkest = drawn.gray.iter().copied().min().unwrap_or(255);
        match darker {
            true => assert!(darkest < 100, "'{typed}' came out at {darkest}"),
            false => assert!(darkest > 150, "'{typed}' came out at {darkest}"),
        }
    }
}

/// No sheet is a question, not a crash.
#[test]
fn stamping_nothing_asks_for_the_sheet() {
    let why = watermark_sheet(&posted(&[("text", None, b"DRAFT")])).unwrap_err();
    assert!(why.contains("printed sheet"), "{why}");
}

/// The page has to offer it, and has to say the one thing about it that is not
/// like a word processor — before somebody prints sixty sheets.
#[test]
fn the_page_offers_the_watermark_and_says_the_toner_goes_on_top() {
    assert!(
        PAGE_BODY.contains("action=\"/watermark\""),
        "the page has no watermark form"
    );
    assert!(
        PAGE_BODY.contains("goes over it"),
        "the page does not say the toner goes on top of the printing"
    );
}

/// A real code comes out, and it is one that decodes.
///
/// Checked by drawing the delta and reading it back rather than by looking at
/// the bytes: a code that is the right shape and cannot be read is exactly the
/// failure this needs to catch, and the only way to catch it is to read it.
#[test]
fn the_browser_writes_a_barcode_that_reads_back() {
    for (kind, text) in [
        ("code128", "INV-2024-00817"),
        ("qr", "https://example.org/renew"),
    ] {
        let request = posted(&[
            ("sheet", Some("form.pdf"), &a_page("Asset register")),
            ("text", None, text.as_bytes()),
            ("kind", None, kind.as_bytes()),
        ]);
        let (bytes, name) = barcode_sheet(&request).expect("it should write a code");
        assert_eq!(name, "barcode.pdf");

        let dir = tempfile::tempdir().unwrap();
        let pdf = dir.path().join("code.pdf");
        std::fs::write(&pdf, &bytes).unwrap();

        // There is ink on the page, and it is black.
        let engine = crate::render::engine().expect("a renderer");
        let document = engine.open(&pdf).expect("it should open");
        let drawn = document.render_gray(0, 300.0).expect("it should draw");
        assert!(
            drawn.gray.iter().any(|level| *level < 60),
            "the {kind} delta came out blank"
        );

        // And a decoder agrees, when there is one on this machine.
        let png = dir.path().join("code.png");
        let image =
            image::GrayImage::from_raw(drawn.width as u32, drawn.height as u32, drawn.gray.clone())
                .expect("the render should be an image");
        image.save(&png).unwrap();
        if let Ok(out) = std::process::Command::new("zbarimg")
            .args(["--nodbus", "--quiet", "--raw"])
            .arg(&png)
            .output()
        {
            let read = String::from_utf8_lossy(&out.stdout).trim_end().to_string();
            if !read.is_empty() {
                assert_eq!(
                    read, text,
                    "a scanner read the {kind} back as something else"
                );
            }
        }
    }
}

/// Nothing to encode is a question, not a crash.
#[test]
fn a_code_of_nothing_is_refused() {
    let why = barcode_sheet(&posted(&[("sheet", Some("f.pdf"), &a_page("x"))])).unwrap_err();
    assert!(why.contains("nothing to put in a code"), "{why}");
    let why = barcode_sheet(&posted(&[("text", None, b"HELLO")])).unwrap_err();
    assert!(why.contains("Choose the sheet"), "{why}");
}

/// A code too big for the paper is refused with the numbers, rather than
/// written half off the page.
#[test]
fn a_code_that_runs_off_the_paper_says_so() {
    let request = posted(&[
        ("sheet", Some("form.pdf"), &a_page("Asset register")),
        (
            "text",
            None,
            b"A REFERENCE LONG ENOUGH TO NOT FIT ON THE PAGE AT ALL",
        ),
        ("kind", None, b"code128"),
        ("module", None, b"2"),
    ]);
    let why = barcode_sheet(&request).unwrap_err();
    assert!(why.contains("runs off"), "{why}");
    assert!(why.contains("mm"), "{why}");
}

/// The page has to offer it, and has to say the thing that decides whether it
/// works: blank paper.
#[test]
fn the_page_offers_the_barcode_and_says_it_needs_blank_paper() {
    assert!(
        PAGE_BODY.contains("action=\"/barcode\""),
        "the page has no barcode form"
    );
    assert!(
        PAGE_BODY.contains("blank paper"),
        "the page does not say a barcode needs blank paper"
    );
    // And that nothing is sent anywhere, which is why it is here rather than on
    // one of the websites that does this.
    assert!(
        PAGE_BODY.contains("Nothing is sent anywhere"),
        "{}",
        "no such line"
    );
}

/// The back really is written on, and the words really are where somebody
/// asked for them once the paper has been turned the way the feed turns it.
#[test]
fn the_browser_writes_on_the_back_either_way_the_paper_comes_back() {
    for (feed, turned) in [("same", false), ("turned", true)] {
        let request = posted(&[
            ("sheet", Some("invoice.pdf"), &a_page("Invoice 2024-8817")),
            ("text", None, b"Terms overleaf"),
            ("feed", None, feed.as_bytes()),
            ("x", None, b"20"),
            ("y", None, b"40"),
        ]);
        let (bytes, name) = the_back_of_the_sheet(&request).expect("it should write a back");
        assert_eq!(name, "back.pdf");

        let dir = tempfile::tempdir().unwrap();
        let pdf = dir.path().join("back.pdf");
        std::fs::write(&pdf, &bytes).unwrap();

        const DPI: f64 = 100.0;
        let engine = crate::render::engine().expect("a renderer");
        let document = engine.open(&pdf).expect("it should open");
        let drawn = document.render_gray(0, DPI).expect("it should draw");
        // What a hand does to the paper, done to the picture of it.
        let seen: Vec<u8> = match turned {
            false => drawn.gray.clone(),
            true => drawn.gray.iter().rev().copied().collect(),
        };
        let mm = |pixels: usize| (pixels as f64 + 0.5) * 25.4 / DPI;
        let spots: Vec<(f64, f64)> = (0..drawn.height)
            .flat_map(|y| (0..drawn.width).map(move |x| (x, y)))
            .filter(|(x, y)| seen[y * drawn.width + x] < 128)
            .map(|(x, y)| (mm(x), mm(y)))
            .collect();
        assert!(!spots.is_empty(), "{feed}: the back came out blank");

        let left = spots.iter().map(|s| s.0).fold(f64::MAX, f64::min);
        let baseline = spots.iter().map(|s| s.1).fold(f64::MIN, f64::max);
        assert!(
            (left - 20.0).abs() < 2.0,
            "{feed}: the words start {left:.1} mm in, not 20"
        );
        assert!(
            (baseline - 40.0).abs() < 2.0,
            "{feed}: the words sit {baseline:.1} mm down, not 40"
        );
    }
}

/// The sheet that answers the question can be had from the browser too, which
/// is the point of it being here: this is the machine with the printer on it.
#[test]
fn the_browser_offers_the_sheet_that_answers_which_way_up() {
    let request = posted(&[
        ("sheet", Some("invoice.pdf"), &a_page("Invoice")),
        ("check", None, b"yes"),
    ]);
    let (bytes, name) = the_back_of_the_sheet(&request).expect("it should write the sheet");
    assert_eq!(name, "which-way-up.pdf");

    let dir = tempfile::tempdir().unwrap();
    let pdf = dir.path().join("which.pdf");
    std::fs::write(&pdf, &bytes).unwrap();
    let engine = crate::render::engine().expect("a renderer");
    let document = engine.open(&pdf).expect("it should open");
    let drawn = document.render_gray(0, 100.0).expect("it should draw");
    let rows: Vec<usize> = (0..drawn.height)
        .filter(|y| (0..drawn.width).any(|x| drawn.gray[y * drawn.width + x] < 128))
        .collect();
    assert!(!rows.is_empty(), "the sheet came out blank");
    // Ink at both ends, so there is a word to read whichever way it comes out.
    let top = *rows.first().unwrap() as f64 * 25.4 / 100.0;
    let bottom = *rows.last().unwrap() as f64 * 25.4 / 100.0;
    assert!(top < 99.0, "nothing near the top: {top:.0} mm");
    assert!(bottom > 198.0, "nothing near the bottom: {bottom:.0} mm");
}

/// Nothing to put on the back is a question, not a crash.
#[test]
fn a_back_with_nothing_on_it_is_refused() {
    let why = the_back_of_the_sheet(&posted(&[("text", None, b"x")])).unwrap_err();
    assert!(why.contains("Choose the printed document"), "{why}");

    let why =
        the_back_of_the_sheet(&posted(&[("sheet", Some("f.pdf"), &a_page("x"))])).unwrap_err();
    assert!(why.contains("nothing to put on the back"), "{why}");
}

/// The page has to offer it, and has to ask the question that decides whether
/// any of it works.
#[test]
fn the_page_offers_the_back_and_asks_which_way_up() {
    assert!(
        PAGE_BODY.contains("action=\"/back\""),
        "the page has no back form"
    );
    assert!(
        PAGE_BODY.contains("Which way up does the back come out?"),
        "the page does not ask the one question that matters"
    );
    // And every feed the form offers is one the program understands.
    for feed in ["same", "turned"] {
        assert!(
            PAGE_BODY.contains(&format!("value=\"{feed}\"")),
            "the form does not offer '{feed}'"
        );
        assert!(crate::duplex::Feed::parse(feed).is_some());
    }
}

/// A stack in, a spreadsheet out, through the browser.
#[test]
fn the_browser_reads_a_stack_back_into_a_spreadsheet() {
    let request = posted(&[
        ("scan", Some("forms.pdf"), &a_page("Name: J. Bezzina")),
        ("fields", None, b"Name"),
    ]);
    let (bytes, name) = harvest_a_stack(&request).expect("it should harvest");
    assert_eq!(name, "harvested.csv");

    let csv = String::from_utf8(bytes).expect("a spreadsheet is text");
    let mut lines = csv.lines();
    assert_eq!(lines.next(), Some("Sheet,Name"));
    let row = lines.next().expect("a row per sheet");
    assert!(row.starts_with("1,"), "{csv}");
    assert!(row.contains("Bezzina"), "{csv}");
}

/// A column of figures is asked for the way the form says to ask, and comes
/// back as figures rather than as rings and letters.
#[test]
fn a_column_of_figures_comes_back_as_figures() {
    let request = posted(&[
        ("scan", Some("forms.pdf"), &a_page("Amount: 240.00")),
        ("fields", None, b"Amount/number"),
    ]);
    let (bytes, _) = harvest_a_stack(&request).expect("it should harvest");
    let csv = String::from_utf8(bytes).unwrap();
    let value = csv
        .lines()
        .nth(1)
        .unwrap()
        .split(',')
        .nth(1)
        .unwrap()
        .to_string();
    assert!(
        value.parse::<f64>().is_ok(),
        "'{value}' is not a figure:\n{csv}"
    );
}

/// No columns is a question with the answer in it.
#[test]
fn a_harvest_with_no_columns_says_what_to_type() {
    let why = harvest_a_stack(&posted(&[("scan", Some("f.pdf"), &a_page("x"))])).unwrap_err();
    assert!(why.contains("at least one column"), "{why}");
    assert!(why.contains("/number"), "{why}");

    let why = harvest_a_stack(&posted(&[("fields", None, b"Name")])).unwrap_err();
    assert!(why.contains("Choose the scanned stack"), "{why}");
}

/// The page has to offer it, and has to say the thing that decides whether it
/// is worth trying at all.
#[test]
fn the_page_offers_the_harvest_and_says_handwriting_is_not_read() {
    assert!(
        PAGE_BODY.contains("action=\"/harvest\""),
        "the page has no harvest form"
    );
    assert!(
        PAGE_BODY.contains("Handwriting is not read"),
        "the page does not say handwriting is not read"
    );
    // And every spelling the form suggests is one the field parser understands.
    for spec in [
        "Amount/number",
        "Address/below",
        "Name=Full name of applicant",
    ] {
        assert!(
            PAGE_BODY.contains(spec),
            "the form does not mention '{spec}'"
        );
        assert!(
            crate::harvest::Field::parse(spec).is_ok(),
            "the form suggests '{spec}', which is not a field"
        );
    }
}

/// The browser refuses a placement off the paper, the same as the command line.
///
/// The check was added to the command and not to the other two interfaces, so
/// for a while the same request written on the web page still wrote a delta with
/// the words off the side of the sheet. One check, three places that need it.
#[test]
fn the_browser_refuses_words_placed_off_the_paper() {
    let request = posted(&[
        ("sheet", Some("letter.pdf"), &a_page("Invoice")),
        ("text", None, b"off the page"),
        ("x", None, b"300"),
        ("y", None, b"400"),
    ]);
    let why = the_back_of_the_sheet(&request).unwrap_err();
    assert!(why.contains("off the"), "{why}");
    assert!(why.contains("300,400"), "it did not say where: {why}");

    // And an ordinary one still goes through.
    let request = posted(&[
        ("sheet", Some("letter.pdf"), &a_page("Invoice")),
        ("text", None, b"Terms overleaf"),
        ("x", None, b"20"),
        ("y", None, b"40"),
    ]);
    assert!(the_back_of_the_sheet(&request).is_ok());
}
