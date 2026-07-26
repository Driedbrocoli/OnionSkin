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
    // The promise is that Onionskin never uses the network. A page that pulls a
    // stylesheet from someone else's server breaks that the moment it is
    // opened, silently, and no test of the Rust would notice.
    let lower = PAGE.to_lowercase();
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
    assert!(PAGE.contains("100%"));
    assert!(PAGE.contains("Fit to page"));
    assert!(PAGE.contains("never uses the network"));
}

#[test]
fn the_page_asks_for_both_documents() {
    assert!(PAGE.contains("name=\"original\""));
    assert!(PAGE.contains("name=\"edited\""));
    assert!(PAGE.contains("enctype=\"multipart/form-data\""));
    assert!(PAGE.contains("action=\"/delta\""));
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

    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response:.300}");
    assert!(response.contains("application/pdf"), "{response:.300}");
    assert!(response.contains("filename=\"delta.pdf\""));
    assert!(response.contains("%PDF"), "the body is not a PDF");
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
