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
