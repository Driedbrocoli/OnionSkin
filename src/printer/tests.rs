//! Tests for talking to a printer.
//!
//! There is no printer here, so most of these stand one up: a socket that
//! speaks just enough IPP or eSCL to answer, and the real client talks to it.
//! That exercises the wire format rather than asserting that the code does what
//! it does — a fake printer that misreads the request fails the test, which is
//! the point.

use super::*;
use std::net::TcpListener;
use std::sync::mpsc;

// ---------------------------------------------------------------------------
// Addresses
// ---------------------------------------------------------------------------

#[test]
fn an_address_comes_apart_the_way_it_is_written() {
    let a = Address::parse("ipp://printer.local/ipp/print", 631, "/ipp/print").unwrap();
    assert_eq!(a.host, "printer.local");
    assert_eq!(a.port, 631);
    assert_eq!(a.path, "/ipp/print");
}

#[test]
fn a_bare_address_gets_the_usual_port_and_path() {
    let a = Address::parse("192.168.1.50", 631, "/ipp/print").unwrap();
    assert_eq!(a.host, "192.168.1.50");
    assert_eq!(a.port, 631);
    assert_eq!(a.path, "/ipp/print");
}

#[test]
fn a_port_is_taken_when_one_is_given() {
    let a = Address::parse("http://printer:8631/eSCL", 80, "/eSCL").unwrap();
    assert_eq!(a.port, 8631);
    assert_eq!(a.path, "/eSCL");
}

#[test]
fn an_ipv6_address_is_not_confused_by_its_own_colons() {
    let a = Address::parse("ipp://[fe80::1]/ipp/print", 631, "/x").unwrap();
    assert_eq!(a.host, "fe80::1");
    assert_eq!(a.port, 631);

    let with_port = Address::parse("ipp://[fe80::1]:9100/ipp/print", 631, "/x").unwrap();
    assert_eq!(with_port.host, "fe80::1");
    assert_eq!(with_port.port, 9100);
    // And it goes back out in brackets, or it is not a URI.
    assert!(
        with_port.ipp_uri().contains("[fe80::1]:9100"),
        "{}",
        with_port.ipp_uri()
    );
}

#[test]
fn the_encrypted_form_says_what_to_do_instead_of_failing_obscurely() {
    let err = Address::parse("ipps://printer/ipp/print", 631, "/x")
        .unwrap_err()
        .to_string();
    assert!(err.contains("encrypted"), "{err}");
    assert!(
        err.contains("631"),
        "it should suggest the plain port: {err}"
    );
}

#[test]
fn nonsense_addresses_are_refused() {
    for bad in [
        "",
        "   ",
        "ipp://",
        "ipp://printer:notaport/x",
        "ipp://[fe80::1/x",
    ] {
        assert!(
            Address::parse(bad, 631, "/x").is_err(),
            "{bad:?} was accepted"
        );
    }
}

#[test]
fn the_default_port_is_left_out_of_the_uri() {
    let a = Address::parse("printer.local", 631, "/ipp/print").unwrap();
    assert_eq!(a.ipp_uri(), "ipp://printer.local/ipp/print");
}

// ---------------------------------------------------------------------------
// The IPP wire format
// ---------------------------------------------------------------------------

#[test]
fn a_request_starts_with_the_bytes_ipp_requires() {
    let mut request = IppRequest::new(operation::PRINT_JOB, 7);
    preamble(&mut request, "ipp://printer/ipp/print", "someone");
    let bytes = request.finish(b"%PDF");

    // version 1.1, operation, request id, then the operation-attributes group.
    assert_eq!(&bytes[0..2], &[0x01, 0x01]);
    assert_eq!(&bytes[2..4], &operation::PRINT_JOB.to_be_bytes());
    assert_eq!(&bytes[4..8], &7u32.to_be_bytes());
    assert_eq!(bytes[8], tag::OPERATION_ATTRIBUTES);

    // The document follows the end-of-attributes tag and nothing else.
    let end = bytes
        .iter()
        .rposition(|b| *b == tag::END_OF_ATTRIBUTES)
        .unwrap();
    assert_eq!(&bytes[end + 1..], b"%PDF");
}

#[test]
fn the_required_attributes_come_first_and_in_order() {
    // IPP is strict about this: charset, then language, then the printer.
    let mut request = IppRequest::new(operation::PRINT_JOB, 1);
    preamble(&mut request, "ipp://printer/ipp/print", "someone");
    let bytes = request.finish(&[]);
    let text = String::from_utf8_lossy(&bytes);

    let charset = text.find("attributes-charset").unwrap();
    let language = text.find("attributes-natural-language").unwrap();
    let uri = text.find("printer-uri").unwrap();
    assert!(charset < language, "charset must come first");
    assert!(language < uri, "the language must precede the printer");
}

#[test]
fn an_attribute_is_encoded_as_tag_name_value() {
    let mut request = IppRequest::new(operation::PRINT_JOB, 1);
    request.text(tag::NAME, "job-name", "Delta");
    let bytes = request.finish(&[]);

    // Straight after the group tag at byte 8.
    assert_eq!(bytes[9], tag::NAME);
    assert_eq!(u16::from_be_bytes([bytes[10], bytes[11]]), 8); // "job-name"
    assert_eq!(&bytes[12..20], b"job-name");
    assert_eq!(u16::from_be_bytes([bytes[20], bytes[21]]), 5); // "Delta"
    assert_eq!(&bytes[22..27], b"Delta");
}

#[test]
fn a_repeated_attribute_is_written_with_an_empty_name() {
    // That is how IPP says "and also this" — a second entry with no key.
    let mut request = IppRequest::new(operation::CUPS_GET_PRINTERS, 1);
    request
        .text(tag::KEYWORD, "requested-attributes", "printer-name")
        .also(tag::KEYWORD, b"printer-state");
    let bytes = request.finish(&[]);

    let reply = parse_ipp(&fake_reply_from(&bytes)).unwrap();
    let names: Vec<&str> = reply
        .attributes
        .iter()
        .filter(|(_, name, _)| name == "requested-attributes")
        .filter_map(|(_, _, value)| value.as_text())
        .collect();
    assert_eq!(names, vec!["printer-name", "printer-state"]);
}

/// Turn a request's bytes into something the reply parser will read, so the
/// encoder can be checked by decoding it.
fn fake_reply_from(request: &[u8]) -> Vec<u8> {
    let mut reply = request.to_vec();
    // A reply has a status where a request has an operation.
    reply[2] = 0x00;
    reply[3] = 0x00;
    reply
}

#[test]
fn a_reply_is_read_back_with_its_status_and_attributes() {
    let mut reply = vec![0x01, 0x01, 0x00, 0x00]; // version, status 0
    reply.extend_from_slice(&1u32.to_be_bytes());
    reply.push(tag::OPERATION_ATTRIBUTES);
    reply.push(tag::JOB_ATTRIBUTES);
    // job-id = 42
    reply.push(tag::INTEGER);
    reply.extend_from_slice(&6u16.to_be_bytes());
    reply.extend_from_slice(b"job-id");
    reply.extend_from_slice(&4u16.to_be_bytes());
    reply.extend_from_slice(&42i32.to_be_bytes());
    reply.push(tag::END_OF_ATTRIBUTES);

    let parsed = parse_ipp(&reply).unwrap();
    assert!(parsed.succeeded());
    assert_eq!(parsed.get("job-id").unwrap().as_integer(), Some(42));
}

#[test]
fn a_refusal_is_reported_in_words_rather_than_a_number() {
    let mut reply = vec![0x01, 0x01, 0x04, 0x0A]; // 0x040A: format unsupported
    reply.extend_from_slice(&1u32.to_be_bytes());
    reply.push(tag::END_OF_ATTRIBUTES);

    let parsed = parse_ipp(&reply).unwrap();
    assert!(!parsed.succeeded());
    let complaint = parsed.complaint();
    assert!(complaint.contains("document format"), "{complaint}");
    assert!(complaint.contains("0x040a"), "{complaint}");
}

#[test]
fn a_truncated_reply_is_reported_rather_than_panicked_on() {
    assert!(parse_ipp(&[0x01, 0x01]).is_err());
    // A reply that stops mid-attribute must stop reading, not index past the
    // end — a printer cut off by a dropped connection does exactly this.
    let mut cut = vec![0x01, 0x01, 0x00, 0x00];
    cut.extend_from_slice(&1u32.to_be_bytes());
    cut.push(tag::JOB_ATTRIBUTES);
    cut.push(tag::INTEGER);
    cut.extend_from_slice(&6u16.to_be_bytes());
    cut.extend_from_slice(b"job"); // the name is short of what it claimed
    assert!(parse_ipp(&cut).is_ok(), "it should stop, not panic");
}

// ---------------------------------------------------------------------------
// HTTP replies
// ---------------------------------------------------------------------------

#[test]
fn a_chunked_reply_is_put_back_together() {
    // Printers send these constantly: they often cannot say how long an answer
    // is until they have finished composing it.
    let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n\
                5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n";
    let response = parse_response(raw).unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(response.body, b"hello world");
}

#[test]
fn a_chunk_header_with_an_extension_on_it_is_still_read() {
    let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n\
                5;name=value\r\nhello\r\n0\r\n\r\n";
    assert_eq!(parse_response(raw).unwrap().body, b"hello");
}

/// A printer that stops talking mid-sentence, which happens when it is
/// switched off, unplugged, or simply out of its depth.
///
/// This used to panic. `at += size + 2` walked over the CRLF that follows a
/// chunk without checking it was there, so a reply cut off after the data left
/// `at` past the end of the buffer and the next turn of the loop sliced from
/// beyond it. The moment it happened is what makes it matter: the document has
/// already gone to the printer by the time the reply is read, so the sheet is
/// moving — and from the window, a panic is reported as "Nothing was written",
/// which is exactly the sentence that gets somebody to feed the sheet again.
#[test]
fn a_chunked_reply_that_stops_in_the_middle_is_reported_rather_than_crashing() {
    for cut in [
        // After the chunk's data, with neither byte of the CRLF.
        &b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nABCD"[..],
        // With one byte of it.
        &b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nABCD\r"[..],
        // In the middle of the data.
        &b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n9\r\nABCD"[..],
        // After a whole chunk, with the next one's header started.
        &b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nABCD\r\n5\r\nab"[..],
    ] {
        let why = parse_response(cut).expect_err("a truncated reply is not a reply");
        let said = why.to_string();
        assert!(said.contains("middle of a chunk"), "{said}");
    }

    // The well-formed one still works, so the bound did not eat the last chunk.
    let whole = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nABCD\r\n0\r\n\r\n";
    assert_eq!(parse_response(whole).unwrap().body, b"ABCD");
}

#[test]
fn a_reply_that_is_not_http_is_reported() {
    assert!(parse_response(b"garbage with no headers").is_err());
    assert!(parse_response(b"not a status line\r\n\r\nbody").is_err());
}

#[test]
fn headers_are_found_whatever_case_they_are_written_in() {
    let raw = b"HTTP/1.1 201 Created\r\nLOCATION: /eSCL/ScanJobs/7\r\n\r\n";
    let response = parse_response(raw).unwrap();
    assert_eq!(response.header("Location"), Some("/eSCL/ScanJobs/7"));
    assert_eq!(response.header("location"), Some("/eSCL/ScanJobs/7"));
}

// ---------------------------------------------------------------------------
// A pretend printer, spoken to over a real socket
// ---------------------------------------------------------------------------

/// Answer exactly one request, and hand the request back down a channel.
fn one_shot(reply: Vec<u8>) -> (String, mpsc::Receiver<Vec<u8>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (send, receive) = mpsc::channel();

    std::thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        // Read the head, then exactly as much body as it says.
        let mut raw = Vec::new();
        let mut byte = [0u8; 1];
        let mut length = 0usize;
        while stream.read_exact(&mut byte).is_ok() {
            raw.push(byte[0]);
            if raw.ends_with(b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&raw).to_lowercase();
                length = head
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .and_then(|value| value.trim().parse().ok())
                    .unwrap_or(0);
                break;
            }
        }
        let mut body = vec![0u8; length];
        if length > 0 {
            let _ = stream.read_exact(&mut body);
        }
        let _ = send.send(body);
        let _ = stream.write_all(&reply);
        let _ = stream.flush();
    });

    (format!("{}:{}", address.ip(), address.port()), receive)
}

/// An IPP reply wrapped in the HTTP a printer would put around it.
fn ipp_http(status: u16, attributes: &[(u8, &str, &[u8])]) -> Vec<u8> {
    let mut ipp = vec![0x01, 0x01];
    ipp.extend_from_slice(&status.to_be_bytes());
    ipp.extend_from_slice(&1u32.to_be_bytes());
    ipp.push(tag::JOB_ATTRIBUTES);
    for (value_tag, name, value) in attributes {
        ipp.push(*value_tag);
        ipp.extend_from_slice(&(name.len() as u16).to_be_bytes());
        ipp.extend_from_slice(name.as_bytes());
        ipp.extend_from_slice(&(value.len() as u16).to_be_bytes());
        ipp.extend_from_slice(value);
    }
    ipp.push(tag::END_OF_ATTRIBUTES);

    let mut http = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/ipp\r\nContent-Length: {}\r\n\r\n",
        ipp.len()
    )
    .into_bytes();
    http.extend_from_slice(&ipp);
    http
}

#[test]
fn a_delta_is_sent_to_a_printer_and_comes_back_with_a_job_number() {
    let (address, sent) = one_shot(ipp_http(
        0x0000,
        &[(tag::INTEGER, "job-id", &42i32.to_be_bytes())],
    ));

    let job = print_bytes(
        &format!("ipp://{address}/ipp/print"),
        b"%PDF-1.4 pretend delta",
        &PrintOptions::default(),
    )
    .unwrap();
    assert_eq!(job, 42);

    // And the printer got a real IPP request with the document on the end.
    let request = sent.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(&request[0..2], &[0x01, 0x01], "not IPP 1.1");
    assert_eq!(&request[2..4], &operation::PRINT_JOB.to_be_bytes());
    assert!(
        request.ends_with(b"%PDF-1.4 pretend delta"),
        "the document was lost"
    );
}

#[test]
fn the_job_tells_the_printer_not_to_scale_the_page() {
    // The whole reason for printing this way rather than through a dialogue: a
    // delta scaled to fit puts every word in the wrong place, and every print
    // dialogue in the world defaults to fitting.
    let (address, sent) = one_shot(ipp_http(0x0000, &[]));
    let _ = print_bytes(
        &format!("ipp://{address}/ipp/print"),
        b"%PDF",
        &PrintOptions::default(),
    );

    let request = sent.recv_timeout(Duration::from_secs(5)).unwrap();
    let text = String::from_utf8_lossy(&request);
    assert!(
        text.contains("print-scaling"),
        "no scaling instruction sent"
    );
    let at = text.find("print-scaling").unwrap();
    assert!(
        text[at..].starts_with("print-scaling\u{0}\u{4}none"),
        "the scaling was not set to none: {:?}",
        &text[at..at + 24.min(text.len() - at)]
    );
}

#[test]
fn copies_and_paper_size_travel_with_the_job() {
    let (address, sent) = one_shot(ipp_http(0x0000, &[]));
    let _ = print_bytes(
        &format!("ipp://{address}/ipp/print"),
        b"%PDF",
        &PrintOptions {
            copies: 3,
            media: Some("iso_a4_210x297mm".into()),
            job_name: "Purchase order".into(),
            two_sided: false,
        },
    );

    let request = sent.recv_timeout(Duration::from_secs(5)).unwrap();
    let text = String::from_utf8_lossy(&request);
    assert!(
        text.contains("iso_a4_210x297mm"),
        "the paper size was dropped"
    );
    assert!(text.contains("Purchase order"), "the job name was dropped");
    assert!(
        text.contains("one-sided"),
        "a delta must not go on the back"
    );
    // Three copies, as a four-byte integer.
    let at = text.find("copies").unwrap();
    assert_eq!(request[at + 6 + 2..at + 6 + 6], 3i32.to_be_bytes());
}

#[test]
fn a_printer_that_refuses_says_why_in_words() {
    let (address, _) = one_shot(ipp_http(0x040A, &[]));
    let err = print_bytes(
        &format!("ipp://{address}/ipp/print"),
        b"%PDF",
        &PrintOptions::default(),
    )
    .unwrap_err()
    .to_string();

    assert!(err.contains("document format"), "{err}");
}

#[test]
fn a_printer_that_is_not_there_says_so_rather_than_hanging() {
    // Port 1 on the loopback: nothing listens there, and the refusal is
    // immediate.
    let err = print_bytes(
        "ipp://127.0.0.1:1/ipp/print",
        b"%PDF",
        &PrintOptions::default(),
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("could not reach"), "{err}");
    assert!(
        err.contains("switched on"),
        "it should suggest the obvious: {err}"
    );
}

#[test]
fn nothing_is_sent_when_there_is_nothing_to_print() {
    let err = print_bytes("ipp://127.0.0.1:1/ipp/print", b"", &PrintOptions::default())
        .unwrap_err()
        .to_string();
    assert!(err.contains("nothing to print"), "{err}");
}

#[test]
fn a_nonsensical_number_of_copies_is_refused_before_anything_is_sent() {
    for copies in [0, 1000] {
        let err = print_bytes(
            "ipp://127.0.0.1:1/ipp/print",
            b"%PDF",
            &PrintOptions {
                copies,
                ..Default::default()
            },
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("copies"), "{copies}: {err}");
    }
}

#[test]
fn a_printer_list_comes_back_as_printers() {
    fn push(ipp: &mut Vec<u8>, value_tag: u8, name: &str, value: &[u8]) {
        ipp.push(value_tag);
        ipp.extend_from_slice(&(name.len() as u16).to_be_bytes());
        ipp.extend_from_slice(name.as_bytes());
        ipp.extend_from_slice(&(value.len() as u16).to_be_bytes());
        ipp.extend_from_slice(value);
    }

    let mut ipp = vec![0x01, 0x01, 0x00, 0x00];
    ipp.extend_from_slice(&1u32.to_be_bytes());
    // Two printers, each opening a printer-attributes group.
    ipp.push(tag::PRINTER_ATTRIBUTES);
    push(&mut ipp, tag::NAME, "printer-name", b"Office");
    push(
        &mut ipp,
        tag::URI,
        "printer-uri-supported",
        b"ipp://box/printers/Office",
    );
    push(
        &mut ipp,
        tag::TEXT,
        "printer-make-and-model",
        b"Brother HL-L2350DW",
    );
    push(&mut ipp, tag::INTEGER, "printer-state", &3i32.to_be_bytes());
    ipp.push(tag::PRINTER_ATTRIBUTES);
    push(&mut ipp, tag::NAME, "printer-name", b"Upstairs");
    push(
        &mut ipp,
        tag::URI,
        "printer-uri-supported",
        b"ipp://box/printers/Upstairs",
    );
    ipp.push(tag::END_OF_ATTRIBUTES);

    let mut http = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", ipp.len()).into_bytes();
    http.extend_from_slice(&ipp);

    let (address, _) = one_shot(http);
    let found = printers(&format!("ipp://{address}/")).unwrap();

    assert_eq!(found.len(), 2, "{found:?}");
    assert_eq!(found[0].name, "Office");
    assert_eq!(found[0].model, "Brother HL-L2350DW");
    assert_eq!(found[0].state, "idle");
    assert_eq!(found[1].name, "Upstairs");
}

// ---------------------------------------------------------------------------
// eSCL — scanning
// ---------------------------------------------------------------------------

#[test]
fn the_scan_request_asks_for_a_plain_picture_of_the_whole_platen() {
    let xml = scan_settings_xml(&ScanRequest::default());

    assert!(xml.contains("<pwg:InputSource>Platen</pwg:InputSource>"));
    assert!(xml.contains("<scan:ColorMode>Grayscale8</scan:ColorMode>"));
    assert!(xml.contains("<scan:XResolution>300</scan:XResolution>"));
    // Nothing that would crop, straighten or turn the page: those throw away
    // the outline the whole workflow is measured from.
    let lower = xml.to_lowercase();
    for automatic in ["autocrop", "deskew", "autoskew", "rotate", "edgeauto"] {
        assert!(
            !lower.contains(automatic),
            "the request asks for {automatic}"
        );
    }
}

#[test]
fn the_scan_area_is_given_in_the_units_escl_measures_in() {
    // Three-hundredths of an inch, whatever resolution the scan is at. Getting
    // this wrong scans a stamp-sized corner of the sheet.
    let xml = scan_settings_xml(&ScanRequest {
        area_mm: Some((210.0, 297.0)),
        ..Default::default()
    });
    // 210 mm is 8.2677 inches, so 2480 of those units.
    assert!(xml.contains("<pwg:Width>2480</pwg:Width>"), "{xml}");
    assert!(xml.contains("<pwg:Height>3508</pwg:Height>"), "{xml}");
}

#[test]
fn colour_and_the_feeder_are_asked_for_when_wanted() {
    let xml = scan_settings_xml(&ScanRequest {
        colour: true,
        feeder: true,
        resolution: 600,
        area_mm: None,
    });
    assert!(xml.contains("RGB24"));
    assert!(xml.contains("<pwg:InputSource>Feeder</pwg:InputSource>"));
    assert!(xml.contains("<scan:XResolution>600</scan:XResolution>"));
}

#[test]
fn a_resolution_no_scanner_has_is_refused_before_anything_is_sent() {
    let dir = tempfile::tempdir().unwrap();
    for dpi in [0, 49, 4800] {
        let err = scan_to(
            "http://127.0.0.1:1/eSCL",
            &ScanRequest {
                resolution: dpi,
                ..Default::default()
            },
            &dir.path().join("scan.jpg"),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("dpi"), "{dpi}: {err}");
    }
}

#[test]
fn a_scanner_hands_back_a_page_and_it_is_written_to_disk() {
    // Two requests: the job, then collecting the page. A one-shot server would
    // answer only the first, so this one answers both.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for (index, stream) in listener.incoming().enumerate().take(2) {
            let Ok(mut stream) = stream else { continue };
            let mut raw = Vec::new();
            let mut byte = [0u8; 1];
            let mut length = 0usize;
            while stream.read_exact(&mut byte).is_ok() {
                raw.push(byte[0]);
                if raw.ends_with(b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&raw).to_lowercase();
                    length = head
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:"))
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(0);
                    break;
                }
            }
            let mut body = vec![0u8; length];
            if length > 0 {
                let _ = stream.read_exact(&mut body);
            }
            let reply: Vec<u8> = if index == 0 {
                b"HTTP/1.1 201 Created\r\nLocation: /eSCL/ScanJobs/7\r\nContent-Length: 0\r\n\r\n"
                    .to_vec()
            } else {
                let page = b"\xff\xd8\xff\xe0 pretend JPEG";
                let mut out = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n",
                    page.len()
                )
                .into_bytes();
                out.extend_from_slice(page);
                out
            };
            let _ = stream.write_all(&reply);
            let _ = stream.flush();
        }
    });

    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("scan.jpg");
    let written = scan_to(
        &format!("http://{}:{}/eSCL", address.ip(), address.port()),
        &ScanRequest::default(),
        &out,
    )
    .unwrap();

    assert_eq!(written, out);
    let bytes = std::fs::read(&out).unwrap();
    assert!(bytes.starts_with(&[0xff, 0xd8]), "that is not a JPEG");
}

#[test]
fn a_scanner_that_is_busy_says_so_in_words() {
    for (status, expected) in [(404u16, "no scanner at"), (409, "busy"), (503, "not ready")] {
        let (address, _) =
            one_shot(format!("HTTP/1.1 {status} No\r\nContent-Length: 0\r\n\r\n").into_bytes());
        let dir = tempfile::tempdir().unwrap();
        let err = scan_to(
            &format!("http://{address}/eSCL"),
            &ScanRequest::default(),
            &dir.path().join("scan.jpg"),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains(expected), "{status}: {err}");
    }
}

#[test]
fn a_scanner_that_accepts_but_says_nowhere_to_collect_is_reported() {
    let (address, _) = one_shot(b"HTTP/1.1 201 Created\r\nContent-Length: 0\r\n\r\n".to_vec());
    let dir = tempfile::tempdir().unwrap();
    let err = scan_to(
        &format!("http://{address}/eSCL"),
        &ScanRequest::default(),
        &dir.path().join("scan.jpg"),
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("did not say where to collect"), "{err}");
}

#[test]
fn a_scanners_capabilities_are_read_off_its_own_reply() {
    let xml = r#"<?xml version="1.0"?>
<scan:ScannerCapabilities xmlns:scan="http://schemas.hp.com/imaging/escl/2011/05/03"
                          xmlns:pwg="http://www.pwg.org/schemas/2010/12/sm">
  <pwg:MakeAndModel>Canon PIXMA TS8350</pwg:MakeAndModel>
  <scan:Platen>
    <scan:PlatenInputCaps>
      <scan:SettingProfiles><scan:SettingProfile>
        <scan:SupportedResolutions><scan:DiscreteResolutions>
          <scan:DiscreteResolution>
            <scan:XResolution>300</scan:XResolution>
            <scan:YResolution>300</scan:YResolution>
          </scan:DiscreteResolution>
          <scan:DiscreteResolution>
            <scan:XResolution>600</scan:XResolution>
            <scan:YResolution>600</scan:YResolution>
          </scan:DiscreteResolution>
        </scan:DiscreteResolutions></scan:SupportedResolutions>
      </scan:SettingProfile></scan:SettingProfiles>
    </scan:PlatenInputCaps>
  </scan:Platen>
</scan:ScannerCapabilities>"#;

    let capabilities = parse_capabilities(xml);
    assert_eq!(capabilities.make_and_model, "Canon PIXMA TS8350");
    assert_eq!(capabilities.resolutions, vec![300, 600]);
    assert!(capabilities.has_platen);
    assert!(!capabilities.has_feeder, "there is no feeder in that reply");
}

#[test]
fn an_element_is_not_confused_by_a_longer_one_ending_the_same_way() {
    // <XResolution> and <MaxXResolution> both end in "XResolution>", and
    // taking the second for the first reports a resolution no scan will use.
    let xml = "<MaxXResolution>1200</MaxXResolution><scan:XResolution>300</scan:XResolution>";
    assert_eq!(xml_values(xml, "XResolution"), vec!["300".to_string()]);
}

#[test]
fn a_reply_that_is_not_xml_gives_nothing_rather_than_nonsense() {
    let capabilities = parse_capabilities("<html><body>404 Not Found</body></html>");
    assert!(capabilities.make_and_model.is_empty());
    assert!(capabilities.resolutions.is_empty());
    assert!(!capabilities.has_platen);
}

#[test]
fn an_address_with_no_scanner_behind_it_is_simply_not_a_scanner() {
    assert!(!scanner_present("http://127.0.0.1:1/eSCL"));
    assert!(!scanner_present("nonsense://"));
}
