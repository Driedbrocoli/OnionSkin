//! A small local web UI, for people who would rather not use a terminal.
//!
//! Deliberately spare. It is an HTTP server written on `std::net` with no
//! framework and no dependency, and it serves exactly one page with no external
//! asset in it — no font from a CDN, no script from anywhere. That is not
//! minimalism for its own sake: Onionskin's promise is that it never uses the
//! network, and a page that fetches a stylesheet from somebody else's server
//! breaks that promise the moment it is opened, silently, in a way no test of
//! this program would catch.
//!
//! It binds to `127.0.0.1`, so it is reachable only from this machine. There is
//! no password: anyone who can reach the address can upload documents and read
//! every delta, which is why binding anywhere else is warned about loudly.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;

use crate::pipeline;
use crate::render::Workspace;

/// The largest upload accepted, in bytes.
///
/// A scan of a long document is a few tens of megabytes; anything past this is
/// a mistake or an attempt to fill the disk, and either way saying so beats
/// finding out when the machine stops.
const MAX_BODY: usize = 128 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum WebError {
    #[error("could not listen on {address}: {source}")]
    Listen {
        address: String,
        source: std::io::Error,
    },
}

/// One parsed request.
struct Request {
    method: String,
    path: String,
    content_type: String,
    /// What the caller said it would take back. A browser says it will take
    /// anything; a script that wants the file says so.
    accept: String,
    body: Vec<u8>,
}

/// One part of a multipart form.
#[derive(Debug)]
struct Part {
    name: String,
    filename: Option<String>,
    data: Vec<u8>,
}

impl Part {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.data).trim().to_string()
    }
}

/// Serve until interrupted.
pub fn serve(host: &str, port: u16) -> Result<(), WebError> {
    let address = format!("{host}:{port}");
    let listener = TcpListener::bind(&address).map_err(|source| WebError::Listen {
        address: address.clone(),
        source,
    })?;

    println!("Onionskin is running at http://{address}/");
    if host != "127.0.0.1" && host != "localhost" && host != "::1" {
        eprintln!(
            "\nwarning: bound to {host}, not just this machine.\n    There is no \
             password. Anyone who can reach that address can upload documents \
             and\n    read every delta made here. Use 127.0.0.1 unless you have a \
             reason not to."
        );
    }
    println!("Press Ctrl-C to stop.");

    for stream in listener.incoming() {
        let Ok(stream) = stream else {
            // A connection that fails to accept is no reason to stop serving
            // the ones behind it.
            continue;
        };
        // One thread per connection, and a panic in one must not take the
        // server down — somebody's next upload should still work.
        std::thread::spawn(move || {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handle(stream)));
        });
    }
    Ok(())
}

fn handle(mut stream: TcpStream) {
    let request = match read_request(&mut stream) {
        Ok(request) => request,
        Err(message) => {
            let _ = respond(
                &mut stream,
                400,
                "text/plain; charset=utf-8",
                message.as_bytes(),
            );
            return;
        }
    };

    let _ = match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/") => respond(
            &mut stream,
            200,
            "text/html; charset=utf-8",
            page().as_bytes(),
        ),
        ("GET", "/health") => respond(&mut stream, 200, "text/plain; charset=utf-8", b"ok"),
        ("POST", "/delta") => match make_delta(&request) {
            // Nothing to say, or a caller that asked for the file rather than
            // a page to read: hand the PDF straight over. The second is what
            // keeps a script working — a browser says it will take anything,
            // and only something automated asks specifically for a PDF.
            Ok((pdf, said, _)) if said.is_empty() || wants_the_file(&request) => {
                respond_file(&mut stream, &pdf, "delta.pdf")
            }
            Ok((pdf, said, took)) => {
                let token = set_aside("delta.pdf", pdf);
                respond(
                    &mut stream,
                    200,
                    "text/html; charset=utf-8",
                    result_page("The delta is ready", &said, &token, "delta.pdf", Some(took))
                        .as_bytes(),
                )
            }
            Err(message) => respond(
                &mut stream,
                422,
                "text/plain; charset=utf-8",
                message.as_bytes(),
            ),
        },
        // Collecting the file the page above offered.
        ("GET", path) if path.starts_with("/get/") => {
            match collect(path.trim_start_matches("/get/")) {
                Some((filename, pdf)) => respond_file(&mut stream, &pdf, &filename),
                None => respond(
                    &mut stream,
                    404,
                    "text/plain; charset=utf-8",
                    b"That file has already been collected. Make it again from the front page.",
                ),
            }
        }
        ("POST", "/join") => match join_files(&request) {
            Ok((bytes, name)) => respond_file(&mut stream, &bytes, &name),
            Err(message) => respond(
                &mut stream,
                422,
                "text/plain; charset=utf-8",
                message.as_bytes(),
            ),
        },
        ("POST", "/harvest") => match harvest_a_stack(&request) {
            Ok((bytes, name)) => respond_file(&mut stream, &bytes, &name),
            Err(message) => respond(
                &mut stream,
                422,
                "text/plain; charset=utf-8",
                message.as_bytes(),
            ),
        },
        ("POST", "/back") => match the_back_of_the_sheet(&request) {
            // A script that asked for the file gets the file. A person gets
            // the instructions first, because printing this one the ordinary
            // way ruins the stack and there is no second attempt.
            Ok((bytes, name, said)) if said.is_empty() || wants_the_file(&request) => {
                respond_file(&mut stream, &bytes, &name)
            }
            Ok((bytes, name, said)) => {
                let token = set_aside(&name, bytes);
                respond(
                    &mut stream,
                    200,
                    "text/html; charset=utf-8",
                    result_page("The backs are ready", &said, &token, &name, None).as_bytes(),
                )
            }
            Err(message) => respond(
                &mut stream,
                422,
                "text/plain; charset=utf-8",
                message.as_bytes(),
            ),
        },
        ("POST", "/barcode") => match barcode_sheet(&request) {
            Ok((bytes, name)) => respond_file(&mut stream, &bytes, &name),
            Err(message) => respond(
                &mut stream,
                422,
                "text/plain; charset=utf-8",
                message.as_bytes(),
            ),
        },
        ("POST", "/watermark") => match watermark_sheet(&request) {
            Ok((bytes, name)) => respond_file(&mut stream, &bytes, &name),
            Err(message) => respond(
                &mut stream,
                422,
                "text/plain; charset=utf-8",
                message.as_bytes(),
            ),
        },
        ("POST", "/convert") => match convert_scan(&request) {
            Ok((bytes, name)) => respond_file(&mut stream, &bytes, &name),
            Err(message) => respond(
                &mut stream,
                422,
                "text/plain; charset=utf-8",
                message.as_bytes(),
            ),
        },
        _ => respond(
            &mut stream,
            404,
            "text/plain; charset=utf-8",
            b"Not found. There is one page, at /",
        ),
    };
}

fn read_request(stream: &mut TcpStream) -> Result<Request, String> {
    let mut reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);

    let mut line = String::new();
    reader
        .read_line(&mut line)
        .map_err(|e| format!("could not read the request: {e}"))?;
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();

    let mut content_length = 0usize;
    let mut content_type = String::new();
    let mut accept = String::new();
    loop {
        let mut header = String::new();
        let read = reader
            .read_line(&mut header)
            .map_err(|e| format!("could not read the headers: {e}"))?;
        if read == 0 || header.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            match name.trim().to_ascii_lowercase().as_str() {
                "content-length" => content_length = value.trim().parse().unwrap_or(0),
                "content-type" => content_type = value.trim().to_string(),
                "accept" => accept = value.trim().to_ascii_lowercase(),
                _ => {}
            }
        }
    }

    // Checked before a byte of it is read, so an upload that would fill the
    // disk is refused rather than accepted and then regretted.
    if content_length > MAX_BODY {
        return Err(format!(
            "that upload is {} MB, and the limit is {} MB.",
            content_length / 1_048_576,
            MAX_BODY / 1_048_576
        ));
    }
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader
            .read_exact(&mut body)
            .map_err(|e| format!("the upload was cut short: {e}"))?;
    }

    Ok(Request {
        method,
        path,
        content_type,
        accept,
        body,
    })
}

/// Split a `multipart/form-data` body into its parts.
fn parse_multipart(content_type: &str, body: &[u8]) -> Result<Vec<Part>, String> {
    let boundary = content_type
        .split(';')
        .map(str::trim)
        .find_map(|piece| piece.strip_prefix("boundary="))
        .map(|b| b.trim_matches('"'))
        .ok_or("the form did not say where one field ends and the next begins")?;

    let separator = format!("--{boundary}").into_bytes();
    let mut parts = Vec::new();
    let mut at = 0usize;

    while let Some(start) = find(body, &separator, at) {
        let from = start + separator.len();
        if body[from..].starts_with(b"--") {
            break; // the closing boundary
        }
        let Some(next) = find(body, &separator, from) else {
            break;
        };
        // Between the boundaries: headers, a blank line, then the data.
        let chunk = &body[from..next];
        let Some(gap) = find(chunk, b"\r\n\r\n", 0) else {
            at = next;
            continue;
        };
        let headers = String::from_utf8_lossy(&chunk[..gap]).to_string();
        // The data ends with the CRLF that precedes the next boundary.
        let data_start = from + gap + 4;
        let data_end = next.saturating_sub(2).max(data_start);
        let data = body[data_start..data_end].to_vec();

        let mut name = String::new();
        let mut filename = None;
        for header in headers.lines() {
            if !header.to_ascii_lowercase().contains("content-disposition") {
                continue;
            }
            for piece in header.split(';').map(str::trim) {
                if let Some(value) = piece.strip_prefix("name=") {
                    name = value.trim_matches('"').to_string();
                } else if let Some(value) = piece.strip_prefix("filename=") {
                    let value = value.trim_matches('"');
                    if !value.is_empty() {
                        filename = Some(value.to_string());
                    }
                }
            }
        }
        parts.push(Part {
            name,
            filename,
            data,
        });
        at = next;
    }
    Ok(parts)
}

fn find(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() || from > haystack.len() - needle.len() {
        return None;
    }
    (from..=haystack.len() - needle.len()).find(|&i| &haystack[i..i + needle.len()] == needle)
}

/// Keep an uploaded file name from reaching anywhere it should not.
///
/// A browser sends whatever the file was called, and a name like
/// `../../.ssh/authorized_keys` is a perfectly ordinary string right up until
/// it is joined onto a path.
fn safe_name(name: &str, fallback: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(fallback);
    let cleaned: String = base
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ' '))
        .collect();
    let cleaned = cleaned.trim_matches(['.', ' ']).to_string();
    if cleaned.is_empty() {
        fallback.to_string()
    } else {
        cleaned
    }
}

/// Whether the caller asked for the delta itself rather than a page about it.
///
/// Everything worth saying about a run is worth saying to a person, and a
/// browser is a person. A script is not: it posted two documents to get a
/// file, and handing it a page of prose instead would break it. The one
/// distinguishes itself from the other by asking for a PDF by name.
fn wants_the_file(request: &Request) -> bool {
    let accept = request.accept.trim();
    accept.starts_with("application/pdf") || accept == "application/octet-stream"
}

/// A finished delta, waiting to be collected.
///
/// The page shows what the run had to say before handing the file over, and a
/// browser cannot be told two things in one response — so the PDF waits here
/// for the second request. It is a hand-off measured in seconds, not a store:
/// collecting takes it, and only the last few are kept.
///
/// No more of a hole than the rest of the server, which has no password: anyone
/// who can reach the address can make a delta and read it. That is why it binds
/// to this machine only, and says so loudly when told to bind elsewhere.
static WAITING: std::sync::Mutex<Vec<(String, String, Vec<u8>)>> =
    std::sync::Mutex::new(Vec::new());

/// How many deltas may be waiting at once. Enough for somebody with three tabs
/// open, and far short of filling memory with documents nobody came back for.
const MOST_WAITING: usize = 4;

/// Put a finished file aside and hand back the name to collect it by.
fn set_aside(filename: &str, pdf: Vec<u8>) -> String {
    let token = unique_token();
    let mut waiting = WAITING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    while waiting.len() >= MOST_WAITING {
        waiting.remove(0);
    }
    waiting.push((token.clone(), filename.to_string(), pdf));
    token
}

/// Collect one, once. The name it should be saved under comes with it.
fn collect(token: &str) -> Option<(String, Vec<u8>)> {
    let mut waiting = WAITING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let at = waiting.iter().position(|(name, _, _)| name == token)?;
    let (_, filename, bytes) = waiting.remove(at);
    Some((filename, bytes))
}

/// A name nothing else will pick, without a random number generator.
fn unique_token() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNT: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_nanos())
        .unwrap_or(0);
    format!(
        "{:x}{:x}",
        nanos,
        COUNT.fetch_add(1, Ordering::Relaxed).wrapping_add(0x9E37)
    )
}

fn make_delta(request: &Request) -> Result<(Vec<u8>, Vec<String>, std::time::Duration), String> {
    let started = std::time::Instant::now();
    let parts = parse_multipart(&request.content_type, &request.body)?;
    let field = |name: &str| parts.iter().find(|p| p.name == name);

    let original = field("original").filter(|p| !p.data.is_empty());
    let edited = field("edited").filter(|p| !p.data.is_empty());
    let (Some(original), Some(edited)) = (original, edited) else {
        return Err(
            "Choose both files: the document as it was printed, and the edited copy.".into(),
        );
    };

    let workspace = Workspace::new(false).map_err(|e| e.to_string())?;
    // Each upload goes in a folder of its own, named after the box it came
    // from. Both used to go in one folder under the name the browser gave —
    // and picking `invoice.pdf` from Documents as the original and
    // `invoice.pdf` from Desktop as the edited copy is the ordinary way people
    // keep a before and an after. The second write landed on the first, so
    // both paths were the same file holding the edited bytes, and Onionskin
    // compared the document with itself. The answer was "the two documents
    // render identically — check you passed the edited file second, and that
    // the edit was saved", every clause of which was untrue, and swapping them
    // gave the same message. A total dead end, over two files with one name.
    let write = |part: &Part, field_name: &str, fallback: &str| -> Result<PathBuf, String> {
        let folder = workspace.path.join(field_name);
        std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
        let path = folder.join(safe_name(
            part.filename.as_deref().unwrap_or(fallback),
            fallback,
        ));
        std::fs::write(&path, &part.data).map_err(|e| e.to_string())?;
        // Working files hold whole documents. Nobody else's business.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(path)
    };
    let original_path = write(original, "original", "original.pdf")?;
    let edited_path = write(edited, "edited", "edited.pdf")?;
    let output = workspace.path.join("delta.pdf");

    // A number the form actually gave, or nothing. The distinction is the
    // whole point: a field left empty must fall through to what this person
    // saved, and a fallback baked in here looks exactly like an answer.
    let number = |name: &str| -> Option<f64> {
        field(name)
            .map(|p| p.text())
            .and_then(|t| t.trim().parse::<f64>().ok())
            .filter(|v| v.is_finite())
    };
    // An unticked checkbox sends nothing at all, so its presence is the answer.
    let ticked = |name: &str| field(name).is_some();

    // Onionskin's own answers, then this machine's saved ones, then whatever
    // was typed on the form. In that order, because it is the order every
    // other interface uses — `Defaults::over` exists to make sure the window
    // and the command line cannot apply a setting differently, and this was
    // the one place that never called it.
    //
    // What it cost: somebody sets a calibration profile once, so that every
    // delta is corrected for their printer's mechanical offset. Made from the
    // browser, it was not — and the sheet comes out two millimetres off, onto
    // paper that was already printed. There is no second attempt at that.
    let mut options = crate::settings::load()
        .defaults
        .over(pipeline::Options::default());
    if let Some(dpi) = number("dpi") {
        options.dpi = dpi;
    }
    if let Some(mode) = field("mode").and_then(|p| pipeline::Mode::parse(&p.text())) {
        options.mode = mode;
    }
    if let Some(margin) = number("margin") {
        options.margin_mm = margin;
    }
    if let Some(profile) = field("profile").map(|p| p.text()).filter(|t| !t.is_empty()) {
        options.profile = Some(profile);
    }
    options.outline = ticked("outline").then(|| {
        let colour = field("outline_colour")
            .map(|p| p.text())
            .unwrap_or_default();
        crate::delta::Outline {
            colour: outline_colour(&colour),
            ..Default::default()
        }
    });

    let outcome = pipeline::run(&original_path, &edited_path, &output, &options)
        .map_err(|e| e.to_string())?;

    if outcome.blocked() {
        let mut message = String::from("Not safe to print onto the existing sheet:\n\n");
        for check in &outcome.checks {
            if check.severity == crate::safety::Severity::Blocker {
                message.push_str(&check.format());
                message.push_str("\n\n");
            }
        }
        return Err(message);
    }

    // Everything the run had to say that did not stop it. The command line
    // prints these and the window shows them; a browser that is handed the PDF
    // and nothing else is the one interface where they would vanish — and one
    // of them is now "Onionskin read the document itself", which is exactly
    // what somebody about to put a printed sheet back in a tray needs.
    let said: Vec<String> = outcome.checks.iter().map(|check| check.format()).collect();
    let pdf = std::fs::read(&output).map_err(|e| e.to_string())?;
    Ok((pdf, said, started.elapsed()))
}

fn respond(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        422 => "Unprocessable Content",
        _ => "Error",
    };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\n\
         Content-Type: {content_type}\r\n\
         Content-Length: {}\r\n\
         Cache-Control: no-store\r\n\
         X-Content-Type-Options: nosniff\r\n\
         Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'; \
         img-src data:; form-action 'self'\r\n\
         Connection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)?;
    stream.flush()
}

fn respond_file(stream: &mut TcpStream, body: &[u8], name: &str) -> std::io::Result<()> {
    // Named from the extension, because a browser handed a .docx labelled as a
    // PDF will offer to open it in a PDF viewer and the person will conclude
    // the file is broken.
    let content_type = match name.rsplit('.').next().unwrap_or("") {
        "pdf" => "application/pdf",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "odt" => "application/vnd.oasis.opendocument.text",
        "onionskin" | "onion" | "json" => "application/json",
        _ => "application/octet-stream",
    };
    write!(
        stream,
        "HTTP/1.1 200 OK\r\n\
         Content-Type: {content_type}\r\n\
         Content-Disposition: attachment; filename=\"{name}\"\r\n\
         Content-Length: {}\r\n\
         Cache-Control: no-store\r\n\
         Connection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)?;
    stream.flush()
}

/// Read a scan and hand back something with a cursor in it.
/// Several PDFs, one after another.
///
/// The one job on this page that needs no options at all: files in, document
/// out. It is here rather than left to the command line because the people who
/// end up on this page are the ones whose machine will not run the window —
/// a server, a container — and a folder of one-page scans with no way to make
/// them into a stack is where the rest of the program stops being reachable.
fn join_files(request: &Request) -> Result<(Vec<u8>, String), String> {
    let parts = parse_multipart(&request.content_type, &request.body)?;
    // Every part called `files`, in the order the browser sent them, which is
    // the order they were chosen.
    let given: Vec<&Part> = parts
        .iter()
        .filter(|part| part.name == "files" && !part.data.is_empty())
        .collect();
    if given.len() < 2 {
        return Err(
            "Choose at least two PDFs. One file is not a join — and the picker \
             takes several at once."
                .into(),
        );
    }

    // On disk, because the joiner opens documents by path.
    let workspace = Workspace::new(false).map_err(|e| e.to_string())?;
    let mut paths = Vec::new();
    for (index, part) in given.iter().enumerate() {
        // Numbered, so they keep the order they arrived in whatever they are
        // called — and named after the original, so the report reads sensibly.
        let named = format!(
            "{index:03}-{}",
            part.filename
                .as_deref()
                .and_then(sanitised)
                .unwrap_or_else(|| format!("part-{index}.pdf"))
        );
        let path = workspace.path.join(named);
        std::fs::write(&path, &part.data).map_err(|e| e.to_string())?;
        paths.push(path);
    }

    let out = workspace.path.join("joined.pdf");
    let joined =
        crate::join::join(&paths, &out, "Onionskin joined document").map_err(|e| e.to_string())?;
    let bytes = std::fs::read(&out).map_err(|e| e.to_string())?;
    let _ = joined;
    Ok((bytes, "joined.pdf".to_string()))
}

/// A stack of filled-in forms, read back into a spreadsheet.
///
/// The reverse of everything else on this page: paper in, data out. It belongs
/// here for the same reason the join does — the people who reach a browser
/// rather than the window are the ones on a server, and "these two hundred forms
/// came back, get them into a spreadsheet" is a request that turns up there.
fn harvest_a_stack(request: &Request) -> Result<(Vec<u8>, String), String> {
    let parts = parse_multipart(&request.content_type, &request.body)?;
    let field = |name: &str| parts.iter().find(|p| p.name == name);

    let Some(scan) = field("scan").filter(|p| !p.data.is_empty()) else {
        return Err("Choose the scanned stack: one PDF with a page per sheet.".into());
    };
    // One field per line, which is how somebody types a list into a form.
    let fields: Vec<crate::harvest::Field> = field("fields")
        .map(|p| p.text())
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(crate::harvest::Field::parse)
        .collect::<Result<_, _>>()?;
    if fields.is_empty() {
        return Err(
            "Name at least one column. A column is the label printed beside it \
             on the form — one to a line, and `Amount/number` for a column of \
             figures."
                .into(),
        );
    }

    let workspace = Workspace::new(false).map_err(|e| e.to_string())?;
    let path = workspace.path.join(safe_name(
        scan.filename.as_deref().unwrap_or("scan.pdf"),
        "scan.pdf",
    ));
    std::fs::write(&path, &scan.data).map_err(|e| e.to_string())?;

    let pages = crate::recipe::pages_in(&path)?;
    let wanted = field("first")
        .map(|p| p.text())
        .and_then(|t| t.parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(pages)
        .min(pages);

    if !crate::typeface::a_face_to_read_with() {
        return Err(
            "There is no font on this machine to read the pages against, so \
             Onionskin cannot find the labels. Install a common face — DejaVu, \
             Liberation — and try again."
                .into(),
        );
    }

    let mut sheets = Vec::with_capacity(wanted);
    for page in 1..=wanted {
        let (gray, registration) = crate::recipe::draw_page(&path, page)?;
        // One unreadable sheet is one bad sheet, not a reason to lose the run.
        let Some((text, _)) = crate::typeface::read_and_match_in(&gray, &registration) else {
            sheets.push(crate::harvest::Sheet::unreadable(page, fields.len()));
            continue;
        };
        sheets.push(crate::harvest::Sheet {
            page,
            values: crate::harvest::pick_from(&text, &fields),
        });
    }

    let harvest = crate::harvest::Harvest { fields, sheets };
    Ok((harvest.csv().into_bytes(), "harvested.csv".to_string()))
}

/// Words on the blank back of a stack that has already been printed.
///
/// The awkward part is the same here as everywhere: which way up the back comes
/// out depends on the printer, and nothing can work it out. So the form asks,
/// and offers to print the sheet that answers it — the machine running this
/// server is the machine with the printer on it, so the answer it remembers is
/// the right one to remember.
fn the_back_of_the_sheet(request: &Request) -> Result<(Vec<u8>, String, Vec<String>), String> {
    let parts = parse_multipart(&request.content_type, &request.body)?;
    let field = |name: &str| parts.iter().find(|p| p.name == name);

    let Some(sheet) = field("sheet").filter(|p| !p.data.is_empty()) else {
        return Err("Choose the printed document whose backs are being written on.".into());
    };
    let checking = field("check").map(|p| p.text()).as_deref() == Some("yes");
    let text = field("text").map(|p| p.text()).unwrap_or_default();
    if text.is_empty() && !checking {
        return Err(
            "There is nothing to put on the back. Say what it should say — or \
             ask for the sheet that finds out which way up the backs come out."
                .into(),
        );
    }
    // What was asked for, then what this machine was told, then the answer most
    // printers give. The middle one matters: `config set feed turned` means
    // "stop asking me", and a web page that quietly went back to asking would
    // be a setting that only half worked.
    let feed = field("feed")
        .map(|p| p.text())
        .and_then(|said| crate::duplex::Feed::parse(&said))
        .or_else(|| {
            crate::settings::load()
                .defaults
                .feed
                .as_deref()
                .and_then(crate::duplex::Feed::parse)
        })
        .unwrap_or_default();
    let two_sided = field("two_sided").map(|p| p.text()).as_deref() == Some("yes");
    let number = |name: &str, fallback: f64| -> f64 {
        field(name)
            .map(|p| p.text())
            .and_then(|t| t.parse::<f64>().ok())
            .filter(|value| value.is_finite() && *value >= 0.0)
            .unwrap_or(fallback)
    };
    let (x_mm, y_mm) = (number("x", 20.0), number("y", 40.0));
    let size_pt = number("size", 12.0);

    let workspace = Workspace::new(false).map_err(|e| e.to_string())?;
    let path = workspace.path.join(safe_name(
        sheet.filename.as_deref().unwrap_or("sheet.pdf"),
        "sheet.pdf",
    ));
    std::fs::write(&path, &sheet.data).map_err(|e| e.to_string())?;

    // The sheet that answers the question, and nothing else on it.
    if checking {
        let paper = crate::recipe::draw_page(&path, 1)?.1.page;
        let out = workspace.path.join("which-way-up.pdf");
        crate::pdf::write_delta(
            &out,
            &[paper],
            &[crate::duplex::a_test_sheet(paper)],
            "Onionskin: which way up",
            None,
        )
        .map_err(|e| e.to_string())?;
        let bytes = std::fs::read(&out).map_err(|e| e.to_string())?;
        return Ok((
            bytes,
            "which-way-up.pdf".to_string(),
            vec![crate::duplex::HOW_TO_USE_THE_TEST_SHEET.to_string()],
        ));
    }

    let pages = crate::recipe::pages_in(&path).unwrap_or(1);
    let sheets = match two_sided {
        true => crate::duplex::sheets_for(pages),
        false => pages,
    };
    let delta_pages = match two_sided {
        true => pages,
        false => sheets,
    };
    let mut sizes = Vec::with_capacity(delta_pages);
    for page in 1..=delta_pages {
        sizes.push(crate::recipe::draw_page(&path, page)?.1.page);
    }

    let mut lines: Vec<Vec<crate::pdf::PlacedLine>> = vec![Vec::new(); delta_pages];
    for sheet in 1..=sheets {
        // A two-sided document has a page for the back and the printer does the
        // turning; a stack going through again is turned by hand.
        let (index, turning) = match two_sided {
            true => (
                crate::duplex::page_of(sheet, crate::duplex::Side::Back),
                crate::duplex::Feed::SameWayUp,
            ),
            false => (sheet, feed),
        };
        if index > delta_pages {
            continue;
        }
        let paper = sizes[index - 1];
        let (x_mm, y_mm, rotation_deg) =
            crate::duplex::turn_a_placement(x_mm, y_mm, 0.0, paper, turning);
        lines[index - 1].push(crate::pdf::PlacedLine {
            text: text.clone(),
            x_mm,
            y_mm,
            size_pt,
            font: crate::pdf::LineFont::Builtin(crate::pdf::Font::Helvetica),
            colour: (0.0, 0.0, 0.0),
            rotation_deg,
        });
    }
    if lines.iter().all(Vec::is_empty) {
        return Err(
            "Nothing landed on any back. A document of one page printed on both \
             sides has no sheet whose back is a page of it."
                .into(),
        );
    }

    // Written straight rather than diffed, so nothing upstream has looked at
    // where the words land. Off the paper prints as a blank sheet.
    for (index, page) in lines.iter().enumerate() {
        for check in
            crate::safety::check_placements(sizes[index], page, crate::safety::DEFAULT_MARGIN_MM)
        {
            if check.severity == crate::safety::Severity::Blocker {
                return Err(check.format());
            }
        }
    }

    let out = workspace.path.join("back.pdf");
    crate::pdf::write_delta(&out, &sizes, &lines, "Onionskin back", None)
        .map_err(|e| e.to_string())?;
    let bytes = std::fs::read(&out).map_err(|e| e.to_string())?;

    // The command line ends with these and the browser used to end with a
    // download and nothing else — the one interface where the instructions
    // vanished, and the one command where losing them costs the whole stack.
    //
    // Two-sided printing is not the default in any print dialogue, so a delta
    // for the backs of a two-sided document, printed the ordinary way, puts
    // every back onto a fresh sheet and leaves the real stack untouched.
    // Printed the other way round, it puts every back upside down on the real
    // stack. Neither is recoverable, and neither is guessable from a file
    // called `back.pdf`.
    //
    // The one-sided case is the same shape: which way up this printer's backs
    // come out is something Onionskin assumed, and saying which way it assumed
    // is what lets somebody notice before the stack goes through.
    let said = match two_sided {
        true => vec![crate::duplex::PRINT_IT_THE_SAME_WAY.to_string()],
        false => vec![
            "Put the printed stack back in the tray the way you would to print the other \
             side, and print this delta onto it."
                .to_string(),
            format!(
                "Placed for a feed of '{}', which means: {}\nIf that is not what this \
                 printer does, every sheet comes out at the wrong end of the paper. \
                 `onionskin back <document> --check` settles it, once.",
                feed.key(),
                feed.describe()
            ),
        ],
    };
    Ok((bytes, "back.pdf".to_string(), said))
}

/// A barcode or a QR code on a sheet, worked out here.
///
/// The reason this is on the page at all: the ordinary way to get a barcode is
/// to type what you want encoded into somebody else's website. An asset number,
/// a patient reference, a case file — those are not things to hand over in
/// exchange for a picture of them, and this machine can work one out in a
/// millisecond without telling anybody.
fn barcode_sheet(request: &Request) -> Result<(Vec<u8>, String), String> {
    let parts = parse_multipart(&request.content_type, &request.body)?;
    let field = |name: &str| parts.iter().find(|p| p.name == name);

    let Some(sheet) = field("sheet").filter(|p| !p.data.is_empty()) else {
        return Err("Choose the sheet the code goes on.".into());
    };
    let text = field("text").map(|p| p.text()).unwrap_or_default();
    if text.is_empty() {
        return Err("There is nothing to put in a code. Say what it should say.".into());
    }
    let qr = field("kind").map(|p| p.text()).as_deref() == Some("qr");
    let number = |name: &str, fallback: f64| -> f64 {
        field(name)
            .map(|p| p.text())
            .and_then(|t| t.parse::<f64>().ok())
            .filter(|value| *value > 0.0)
            .unwrap_or(fallback)
    };
    let module_mm = number("module", if qr { 0.8 } else { 0.4 });
    let height_mm = number("height", 15.0);
    let x_mm = number("x", 20.0);
    let y_mm = number("y", 40.0);

    let symbol = match qr {
        true => {
            let level = field("level")
                .map(|p| p.text())
                .and_then(|t| crate::barcode::qr::Ecc::parse(&t))
                .unwrap_or(crate::barcode::qr::Ecc::Medium);
            crate::barcode::qr::encode(&text, level).map_err(|e| e.to_string())?
        }
        false => crate::barcode::code128::encode(&text).map_err(|e| e.to_string())?,
    };

    let workspace = Workspace::new(false).map_err(|e| e.to_string())?;
    let path = workspace.path.join(safe_name(
        sheet.filename.as_deref().unwrap_or("sheet.pdf"),
        "sheet.pdf",
    ));
    std::fs::write(&path, &sheet.data).map_err(|e| e.to_string())?;

    let pages = crate::recipe::pages_in(&path).unwrap_or(1);
    let mut sizes = Vec::with_capacity(pages);
    for page in 1..=pages {
        sizes.push(crate::recipe::draw_page(&path, page)?.1.page);
    }

    let across_mm = symbol.width_mm(module_mm);
    let down_mm = match qr {
        true => symbol.height_mm(module_mm),
        false => height_mm + symbol.quiet as f64 * module_mm * 2.0,
    };
    if x_mm + across_mm > sizes[0].width_mm || y_mm + down_mm > sizes[0].height_mm {
        return Err(format!(
            "That code comes to {across_mm:.0} x {down_mm:.0} mm, and at \
             {x_mm:.0},{y_mm:.0} it runs off a {} sheet. Move it, or make the \
             modules smaller.",
            sizes[0].describe()
        ));
    }

    let mut shapes: Vec<Vec<crate::pdf::PlacedShape>> = vec![Vec::new(); pages];
    let boxes = match qr {
        true => symbol.rectangles(module_mm),
        false => symbol.bars(module_mm, height_mm),
    };
    for (bx, by, width_mm, height_mm) in boxes {
        shapes[0].push(crate::pdf::PlacedShape {
            drawing: crate::pdf::Drawing::Rect {
                x_mm: x_mm + bx,
                y_mm: y_mm + by,
                width_mm,
                height_mm,
                radius_mm: 0.0,
            },
            stroke: None,
            fill: Some((0.0, 0.0, 0.0)),
            width_mm: 0.0,
            dash_mm: None,
        });
    }

    let out = workspace.path.join("barcode.pdf");
    let blank: Vec<Vec<crate::pdf::PlacedLine>> = vec![Vec::new(); pages];
    crate::pdf::write_page_content(&out, &sizes, &blank, &shapes, "Onionskin code", None)
        .map_err(|e| e.to_string())?;
    let bytes = std::fs::read(&out).map_err(|e| e.to_string())?;
    Ok((bytes, "barcode.pdf".to_string()))
}

/// A word across every page of a sheet that is already printed.
///
/// The other job here that needs no measuring: a document goes in, a delta with
/// DRAFT across it comes out. It belongs on this page for the same reason the
/// join does — the people who reach the browser are the ones without the
/// window, and "stamp this DRAFT before it goes out" is the request that turns
/// up on a server rather than on a desk.
fn watermark_sheet(request: &Request) -> Result<(Vec<u8>, String), String> {
    let parts = parse_multipart(&request.content_type, &request.body)?;
    let field = |name: &str| parts.iter().find(|p| p.name == name);

    let Some(sheet) = field("sheet").filter(|p| !p.data.is_empty()) else {
        return Err("Choose the printed sheet to mark.".into());
    };
    let text = field("text")
        .map(|p| p.text())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| "DRAFT".to_string());
    // A grey out of range is brought back into it rather than refused: the
    // placement clamps anyway, and a form that rejects "80" for not being "0.8"
    // is a form that teaches nothing.
    let grey = field("grey")
        .map(|p| p.text())
        .filter(|t| !t.is_empty())
        .and_then(|t| t.parse::<f64>().ok())
        .map(|given| if given > 1.0 { given / 100.0 } else { given });

    let workspace = Workspace::new(false).map_err(|e| e.to_string())?;
    let path = workspace.path.join(safe_name(
        sheet.filename.as_deref().unwrap_or("sheet.pdf"),
        "sheet.pdf",
    ));
    std::fs::write(&path, &sheet.data).map_err(|e| e.to_string())?;

    let pages = crate::recipe::pages_in(&path).unwrap_or(1);
    let font = crate::pdf::Font::Helvetica;
    let mut sizes = Vec::with_capacity(pages);
    let mut lines: Vec<Vec<crate::pdf::PlacedLine>> = vec![Vec::new(); pages];
    for page in 1..=pages {
        let paper = crate::recipe::draw_page(&path, page)?.1.page;
        sizes.push(paper);
        let Some(mark) = crate::watermark::across(&text, paper, font, None, grey) else {
            return Err("There is nothing to write across the sheet.".into());
        };
        lines[page - 1].push(crate::pdf::PlacedLine {
            text: mark.text.clone(),
            x_mm: mark.x_mm,
            y_mm: mark.y_mm,
            size_pt: mark.size_pt,
            font: crate::pdf::LineFont::Builtin(font),
            colour: mark.colour(),
            rotation_deg: mark.rotation_deg,
        });
    }

    let out = workspace.path.join("watermark.pdf");
    crate::pdf::write_delta(&out, &sizes, &lines, "Onionskin watermark", None)
        .map_err(|e| e.to_string())?;
    let bytes = std::fs::read(&out).map_err(|e| e.to_string())?;
    Ok((bytes, "watermark.pdf".to_string()))
}

/// A filename safe to write into a scratch folder.
///
/// A browser sends whatever the file was called, and what it was called may be
/// `../../etc/passwd`. Only the last component is kept, and only the parts of
/// it that are plainly a name.
fn sanitised(given: &str) -> Option<String> {
    let last = given.rsplit(['/', '\\']).next()?;
    let kept: String = last
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        .collect();
    let kept = kept.trim_matches('.').to_string();
    (!kept.is_empty()).then_some(kept)
}

fn convert_scan(request: &Request) -> Result<(Vec<u8>, String), String> {
    use crate::letters::{read_with_font, ReadOptions};
    use crate::office::{self, Format, Layout};
    use crate::scan::{register, ScanOptions};

    let parts = parse_multipart(&request.content_type, &request.body)?;
    let field = |name: &str| parts.iter().find(|p| p.name == name);

    let Some(scan) = field("scan").filter(|p| !p.data.is_empty()) else {
        return Err("Choose a scan to read: a PDF, or a PNG, JPEG, TIFF or BMP.".into());
    };
    // Optional. The command line stopped asking for a font when it learned to
    // work out which face a page is set in, and the browser kept asking —
    // sending people to hunt through C:\Windows\Fonts for a question the page
    // can answer itself.
    let font_part = field("font").filter(|p| !p.data.is_empty());

    let page = field("page")
        .map(|p| p.text())
        .filter(|t| !t.is_empty())
        .map(|t| crate::geometry::parse_page(&t))
        .transpose()
        .map_err(|e| e.to_string())?
        .unwrap_or(crate::geometry::PageSize {
            width_mm: 210.0,
            height_mm: 297.0,
        });

    // A file on disk either way: the renderer opens documents by path, and the
    // font loader memory-maps the programme rather than copying it about.
    let workspace = Workspace::new(false).map_err(|e| e.to_string())?;

    // A PDF is what every multifunction printer produces by default, so it is
    // the commonest thing to be handed — and it needs no finding, because a
    // document says where its own edges are.
    let looks_like_a_document = scan.data.starts_with(b"%PDF");
    let (gray, registration) = if looks_like_a_document {
        let path = workspace.path.join(safe_name(
            scan.filename.as_deref().unwrap_or("scan.pdf"),
            "scan.pdf",
        ));
        std::fs::write(&path, &scan.data).map_err(|e| e.to_string())?;
        crate::recipe::draw_page(&path, 1)?
    } else {
        let image = image::load_from_memory(&scan.data)
            .map_err(|e| format!("That does not look like a scan Onionskin can read: {e}"))?;
        let registration = register(&image, ScanOptions::new(page)).map_err(|e| e.to_string())?;
        (image.to_luma8(), registration)
    };
    // The page turned out to be whatever it turned out to be. For a picture
    // that is the size given; for a PDF the document said so itself.
    let page = registration.page;

    let text = match &font_part {
        Some(part) => {
            let font_path = workspace.path.join(safe_name(
                part.filename.as_deref().unwrap_or("font.ttf"),
                "font.ttf",
            ));
            std::fs::write(&font_path, &part.data).map_err(|e| e.to_string())?;
            let font = crate::font::EmbeddedFont::load(&font_path).map_err(|e| e.to_string())?;
            read_with_font(&gray, &registration, &ReadOptions::default(), &font, None)
                .map_err(|e| e.to_string())?
        }
        // No font given, so the page is asked which one it is set in — the
        // same answer the command line works out, from the same code.
        None => match crate::typeface::read_and_match_in(&gray, &registration) {
            Some((text, _)) => text,
            None => {
                return Err(
                    "There is no font on this machine to read the page against, so \
                     Onionskin can see where the ink is but not what it says.\n\n\
                     Install a common face — DejaVu, Liberation — or choose the font \
                     the page was set in below. Any .ttf or .otf file will do."
                        .into(),
                )
            }
        },
    };
    if text.lines.is_empty() {
        return Err(
            "No writing was found on that scan.\n\nCheck the paper size is right, \
             and that the scan is of the whole sheet rather than a crop of part \
             of it."
                .into(),
        );
    }

    let document = office::document_from_page(&text, page).map_err(|e| e.to_string())?;
    let layout = match field("layout").map(|p| p.text()).as_deref() {
        Some("flow") => Layout::Flow,
        _ => Layout::Placed,
    };

    let wanted = field("format").map(|p| p.text()).unwrap_or_default();
    match Format::parse(&wanted) {
        Some(format) => {
            let bytes = office::write(&document, format, layout).map_err(|e| e.to_string())?;
            Ok((bytes, format!("page.{}", format.extension())))
        }
        None => {
            // An Onionskin document, which every other command here takes.
            let json = serde_json::to_vec_pretty(&document).map_err(|e| e.to_string())?;
            Ok((json, "page.onionskin".to_string()))
        }
    }
}

/// The one page. No script, no external asset, nothing to fetch.
/// The one page this server serves.
///
/// The settings boxes are filled in from what this machine has saved, rather
/// than from numbers written into the HTML. A browser sends every box on the
/// form whether or not anybody touched it, so a form pre-filled with
/// Onionskin's own answers *is* the setting — it overwrites the person's on
/// every single run, and the calibration profile they set once is thrown away
/// every time they use the browser instead of the terminal. The sheet then
/// comes out a couple of millimetres off, onto paper that is already printed.
fn page() -> String {
    let mine = crate::settings::load().defaults;
    let mode = mine.mode.as_deref().unwrap_or("raster");
    let saved = PAGE_BODY
        .replace(
            "{{raster}}",
            if mode == "vector" { "" } else { " selected" },
        )
        .replace(
            "{{vector}}",
            if mode == "vector" { " selected" } else { "" },
        )
        .replace(
            "{{dpi}}",
            &trim_number(mine.dpi.unwrap_or(pipeline::DEFAULT_DPI)),
        )
        .replace(
            "{{margin}}",
            &trim_number(mine.margin_mm.unwrap_or(crate::safety::DEFAULT_MARGIN_MM)),
        )
        .replace(
            "{{profile}}",
            &escape_html(mine.profile.as_deref().unwrap_or("")),
        )
        .replace(
            "{{outline}}",
            if mine.outline.unwrap_or(false) {
                " checked"
            } else {
                ""
            },
        )
        .replace("{{whose}}", &whose_settings(&mine));
    format!("{HEAD}{saved}")
}

/// A number as somebody would write it: `400`, not `400.0`.
///
/// The boxes it fills are `step="any"` rather than stepped to 50 and 0.5,
/// because they now show whatever this machine has saved. A browser refuses to
/// submit a number field whose value is off its own step grid, so a saved
/// resolution of 437 would have left somebody with a form that would not send
/// and no obvious reason why.
fn trim_number(value: f64) -> String {
    let text = format!("{value}");
    text.strip_suffix(".0").unwrap_or(&text).to_string()
}

/// Whether the boxes above are this machine's settings or Onionskin's answers.
///
/// Said out loud because the difference matters and is invisible otherwise: a
/// person who set a calibration profile has to be able to see that it is being
/// used, and a person who did not has to be able to see that there is one to
/// set.
fn whose_settings(mine: &crate::settings::Defaults) -> String {
    let set = mine.dpi.is_some()
        || mine.margin_mm.is_some()
        || mine.mode.is_some()
        || mine.profile.is_some()
        || mine.outline.is_some();
    if set {
        "Filled in from this machine's settings. Change them here for one run, or with \
         <code>onionskin config set</code> for every run."
            .to_string()
    } else {
        "Onionskin's own answers. <code>onionskin config set profile NAME</code> makes your \
         printer's calibration the one used here every time."
            .to_string()
    }
}

/// What a finished run had to say, and a way to fetch the delta.
///
/// A browser is handed one thing per request, so a run with something worth
/// saying says it here and offers the file second. Nothing is lost by that: the
/// warnings are the reason a person would not print the delta at all, and a
/// file that arrives before them arrives too late to be worth reading.
fn result_page(
    title: &str,
    said: &[String],
    token: &str,
    filename: &str,
    took: Option<std::time::Duration>,
) -> String {
    let mut lines = String::new();
    for check in said {
        // A check is a first line and an indented detail under it, which is how
        // the command line prints them too.
        let mut parts = check.splitn(2, '\n');
        let first = parts.next().unwrap_or_default();
        let rest = parts.next().unwrap_or("").trim();
        let loud = first.starts_with("WARNING") || first.starts_with("BLOCKER");
        lines.push_str(&format!(
            "<div class=\"{}\"><p><strong>{}</strong></p>{}</div>\n",
            if loud { "warn" } else { "note" },
            escape_html(first),
            if rest.is_empty() {
                String::new()
            } else {
                format!("<p class=\"hint\">{}</p>", escape_html(rest))
            }
        ));
    }

    // How long it took. A browser shows nothing at all while it waits, so
    // afterwards is the only chance to say that the waiting was the work.
    let spent = match took.map(|took| took.as_secs_f64()) {
        Some(seconds) if seconds >= 60.0 => format!(" Took {:.0} minutes.", seconds / 60.0),
        Some(seconds) if seconds >= 1.0 => format!(" Took {seconds:.0} seconds."),
        _ => String::new(),
    };

    format!(
        "{HEAD}\n<h1>{title}</h1>\n\
         <p class=\"lede\">Worth reading before you print it.{spent}</p>\n\
         {lines}\n\
         <p><a class=\"get\" href=\"/get/{token}\" download=\"{filename}\">\
         Download {filename}</a></p>\n\
         <h2>Printing it</h2>\n\
         <ol>\n\
         <li>Put the printed sheet back in the tray. Check which way up, and \
         which end goes in first.</li>\n\
         <li>Print at 100% — turn <em>off</em> \"Fit to page\", which scales by a \
         few percent and lines nothing up.</li>\n\
         <li>Do one sheet and hold it against the original before committing \
         any more.</li>\n\
         </ol>\n\
         <p><a href=\"/\">Make another</a></p>\n\
         <footer>Onionskin never sends your documents anywhere. Everything here happened on \
         this machine.</footer>\n\
         </body>\n</html>\n"
    )
}

/// The five characters HTML cannot take literally. A document called
/// `Smith &amp; Sons <draft>` would otherwise close a tag nobody opened.
/// A box colour by the name the form offers.
///
/// The same six the window offers, and red when the name is anything else —
/// a form can be posted by hand with any value in it, and an unknown colour is
/// not worth refusing a delta over.
fn outline_colour(name: &str) -> (f64, f64, f64) {
    match name.trim().to_ascii_lowercase().as_str() {
        "blue" => (0.10, 0.30, 0.85),
        "green" => (0.00, 0.55, 0.20),
        "orange" => (0.95, 0.45, 0.00),
        "magenta" => (0.85, 0.10, 0.60),
        "black" => (0.0, 0.0, 0.0),
        _ => (0.80, 0.10, 0.10),
    }
}

fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

const HEAD: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Onionskin</title>
<style>
  :root { color-scheme: light dark; }
  body {
    font: 16px/1.55 system-ui, -apple-system, "Segoe UI", Roboto, sans-serif;
    max-width: 44rem; margin: 0 auto; padding: 2rem 1.25rem 4rem;
  }
  h1 { font-size: 1.6rem; margin-bottom: .2rem; }
  h2 { font-size: 1.15rem; margin-top: 2rem; }
  p.lede { margin-top: 0; opacity: .75; }
  fieldset { border: 1px solid rgba(128,128,128,.4); border-radius: .5rem;
             padding: 1rem 1.1rem 1.2rem; margin: 1.5rem 0; }
  legend { padding: 0 .4rem; font-weight: 600; }
  label { display: block; margin: .9rem 0 .25rem; font-weight: 600; }
  .hint { font-weight: 400; opacity: .7; font-size: .9rem; }
  input, select { font: inherit; padding: .4rem; width: 100%; box-sizing: border-box; }
  .row { display: flex; gap: 1rem; flex-wrap: wrap; }
  .row > div { flex: 1 1 9rem; }
  button { font: inherit; font-weight: 600; padding: .6rem 1.4rem;
           margin-top: 1.2rem; border-radius: .4rem; cursor: pointer; }
  .warn { border-left: 3px solid #d63333; padding-left: .9rem; margin: 1.5rem 0; }
  footer { margin-top: 2.5rem; font-size: .9rem; opacity: .7; }
  code { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: .9em; }
  .note { border-left: 3px solid rgba(128,128,128,.5); padding-left: .9rem; margin: 1.2rem 0; }
  .note p, .warn p { margin: .3rem 0; }
  a.get { display: inline-block; font-weight: 600; padding: .6rem 1.4rem; margin: 1.4rem 0;
          border: 1px solid rgba(128,128,128,.5); border-radius: .4rem; text-decoration: none; }
</style>
</head>
<body>
"#;

const PAGE_BODY: &str = r#"
<h1>Onionskin</h1>
<p class="lede">Add words to a page that is already printed.</p>

<form method="post" action="/delta" enctype="multipart/form-data">
  <fieldset>
    <legend>The two documents</legend>

    <label for="original">The document as it was printed
      <span class="hint">— .pdf, .docx, .odt and plain text need nothing
      installed. .doc, .rtf, spreadsheets and slides need LibreOffice.</span></label>
    <input type="file" id="original" name="original" required>

    <label for="edited">The edited copy</label>
    <input type="file" id="edited" name="edited" required>
    <p class="hint">A long document takes a while — a page a second or so, and
    the browser shows nothing until it is done.</p>
  </fieldset>

  <fieldset>
    <legend>Settings</legend>
    <div class="row">
      <div>
        <label for="mode">Delta</label>
        <select id="mode" name="mode">
          <option value="raster"{{raster}}>Raster — exactly the new pixels</option>
          <option value="vector"{{vector}}>Vector — sharper, clips to boxes</option>
        </select>
      </div>
      <div>
        <label for="dpi">Resolution</label>
        <input type="number" id="dpi" name="dpi" value="{{dpi}}" min="50" max="1200" step="any">
      </div>
      <div>
        <label for="margin">Edge margin (mm)</label>
        <input type="number" id="margin" name="margin" value="{{margin}}" min="0" max="40" step="any">
      </div>
      <div>
        <label for="profile">Printer profile
          <span class="hint">— optional</span></label>
        <input type="text" id="profile" name="profile" value="{{profile}}" placeholder="office">
      </div>
    </div>
    <p class="hint">{{whose}}</p>
    <p>
      <label for="outline">
        <input type="checkbox" id="outline" name="outline" value="yes"{{outline}}>
        Draw a box round every change, so it is easy to see
      </label>
      <span class="hint">— the box is printed onto the paper too</span>
    </p>
    <div class="row">
      <div>
        <label for="outline_colour">Box colour</label>
        <select id="outline_colour" name="outline_colour">
          <option value="red">Red</option>
          <option value="blue">Blue</option>
          <option value="green">Green</option>
          <option value="orange">Orange</option>
          <option value="magenta">Magenta</option>
          <option value="black">Black</option>
        </select>
      </div>
    </div>
  </fieldset>

  <button type="submit">Make the delta</button>
</form>

<h2>Turn a scan into something you can edit</h2>
<p>
  Read the letters off a scan and get back a Word document, an OpenDocument
  text, or an Onionskin document — each line where it was found on the paper.
</p>

<form method="post" action="/convert" enctype="multipart/form-data">
  <fieldset>
    <legend>The scan</legend>

    <label for="scan">The scanned page
      <span class="hint">— a .pdf straight off the scanner, or a picture of
      the page: .png, .jpg, .tiff, .bmp</span></label>
    <input type="file" id="scan" name="scan" accept=".pdf,image/*" required>

    <label for="font">The font the page was set in
      <span class="hint">— optional. Onionskin works out which face the page
      is set in. Give one only for an alphabet the built-in faces do not
      cover. .ttf, .otf or .ttc.</span></label>
    <input type="file" id="font" name="font" accept=".ttf,.otf,.ttc">

    <div class="row">
      <div>
        <label for="format">Write it as</label>
        <select id="format" name="format">
          <option value="docx">Word — .docx</option>
          <option value="odt">LibreOffice — .odt</option>
          <option value="onionskin">Onionskin document</option>
        </select>
      </div>
      <div>
        <label for="layout">Lay it out</label>
        <select id="layout" name="layout">
          <option value="placed">Where it was on the paper</option>
          <option value="flow">As ordinary paragraphs</option>
        </select>
      </div>
      <div>
        <label for="page">Paper
          <span class="hint">— not needed for a PDF</span></label>
        <input type="text" id="page" name="page" value="a4" placeholder="a4">
      </div>
    </div>
  </fieldset>

  <button type="submit">Read it</button>
</form>

<h2>Put several PDFs into one</h2>
<p>
  A flatbed gives one file per sheet, not one file for the stack. Twenty sheets
  scanned is twenty files, and almost everything else here — and on the command
  line — wants one document with twenty pages in it.
</p>

<form method="post" action="/join" enctype="multipart/form-data">
  <fieldset>
    <legend>The files</legend>

    <label for="files">The PDFs, in the order their pages should come out
      <span class="hint">— choose several at once. Mixed paper is fine: each
      page keeps its own size.</span></label>
    <input type="file" id="files" name="files" accept=".pdf" multiple required>
  </fieldset>

  <button type="submit">Join them</button>
</form>
<p class="hint">
  A file picker sorts <code>page-10</code> before <code>page-2</code>, and a
  stack in that order is a stack in the wrong order. Rename them
  <code>page-01</code>, <code>page-02</code>, <code>page-10</code> and it comes
  out right by itself.
</p>

<h2>Filled-in forms, back into a spreadsheet</h2>
<p>
  The other direction. The sheets came back filled in, and this reads them
  instead of somebody typing them. A column is named after the label printed
  beside it on the form, and the value is whatever follows that label — stopping
  where the next label starts, which is what keeps two fields on one line from
  running into each other.
</p>

<form method="post" action="/harvest" enctype="multipart/form-data">
  <fieldset>
    <legend>The stack</legend>

    <label for="hvscan">The scanned stack
      <span class="hint">— one PDF with a page per sheet. Most scanners will make
      one; a folder of separate pictures will not do.</span></label>
    <input type="file" id="hvscan" name="scan"
      accept=".pdf,.docx,.doc,.odt,.rtf" required>

    <label for="fields">The columns, one to a line
      <span class="hint">— <code>Amount/number</code> for a column of figures,
      <code>Address/below</code> where the value sits under its caption, and
      <code>Name=Full name of applicant</code> to give a long label a short
      heading.</span></label>
    <textarea id="fields" name="fields" rows="5"
      placeholder="Name&#10;Date&#10;Amount/number" required></textarea>

    <label for="first">Read only the first few sheets
      <span class="hint">— leave blank for all of them.</span></label>
    <input type="number" id="first" name="first" min="1" max="9999">
  </fieldset>

  <button type="submit">Read the stack</button>
</form>
<p class="hint">
  Handwriting is not read. Onionskin matches printed letter shapes against the
  fonts on this machine, and a signature has nothing to match — so a hand-filled
  form comes back mostly empty, and that is the honest answer rather than a guess.
</p>

<h2>The back of the sheet</h2>
<p>
  Terms on the back of an invoice, an address on the reverse of a compliment slip,
  &quot;continued overleaf&quot; on a letter. The position is measured
  <strong>on the back as you will look at it</strong>, holding the sheet the right
  way up.
</p>

<form method="post" action="/back" enctype="multipart/form-data">
  <fieldset>
    <legend>The document</legend>

    <label for="bksheet">The printed document</label>
    <input type="file" id="bksheet" name="sheet"
      accept=".pdf,.docx,.doc,.odt,.rtf" required>

    <label for="bktext">What goes on the back</label>
    <input type="text" id="bktext" name="text"
      placeholder="Terms of payment overleaf" maxlength="500">

    <label for="bkx">How far in from the left, in millimetres</label>
    <input type="number" id="bkx" name="x" value="20" min="0" max="2000" step="1">

    <label for="bky">How far down from the top, in millimetres</label>
    <input type="number" id="bky" name="y" value="40" min="0" max="2000" step="1">

    <label for="feed">Which way up does the back come out?</label>
    <select id="feed" name="feed">
      <option value="" selected>Whatever this machine was told</option>
      <option value="same">Same way up — turn it over like a book page and it reads upright</option>
      <option value="turned">Turned around — the top is at the other end of the paper</option>
    </select>

    <label for="two_sided">Already printed on both sides</label>
    <select id="two_sided" name="two_sided">
      <option value="no">No — the backs are blank</option>
      <option value="yes">Yes — every back is a page of it</option>
    </select>
  </fieldset>

  <button type="submit">Write the delta</button>
</form>

<form method="post" action="/back" enctype="multipart/form-data">
  <fieldset>
    <legend>Or find out which way up the backs come out</legend>
    <label for="cksheet">The printed document, for its paper size</label>
    <input type="file" id="cksheet" name="sheet"
      accept=".pdf,.docx,.doc,.odt,.rtf" required>
    <input type="hidden" name="check" value="yes">
  </fieldset>
  <button type="submit">Print a sheet and find out</button>
</form>
<p class="hint">
  One word at each end of the paper. Print it on the back of one sheet, hold the
  sheet with the front the right way up, and turn it over sideways like the page
  of a book. Whichever word is now at the top is the answer. Nobody knows this
  about their own printer, and guessing puts every sheet in the run at the wrong
  end of the paper.
</p>

<h2>A barcode or a QR code</h2>
<p>
  Worked out on this machine. Nothing is sent anywhere — which is the point,
  because the ordinary way to get a barcode is to type the thing you want encoded
  into somebody else&#39;s website, and an asset number or a case reference is not
  a thing to hand over in exchange for a picture of it.
</p>

<form method="post" action="/barcode" enctype="multipart/form-data">
  <fieldset>
    <legend>The code</legend>

    <label for="bcsheet">The sheet it goes on</label>
    <input type="file" id="bcsheet" name="sheet"
      accept=".pdf,.docx,.doc,.odt,.rtf" required>

    <label for="bctext">What it says</label>
    <input type="text" id="bctext" name="text" placeholder="INV-2024-00817"
      maxlength="500" required>

    <label for="kind">Which kind</label>
    <select id="kind" name="kind">
      <option value="code128">Barcode — bars, read by a handheld scanner</option>
      <option value="qr">QR code — a square, read by a telephone</option>
    </select>

    <label for="bcx">How far in from the left, in millimetres</label>
    <input type="number" id="bcx" name="x" value="20" min="0" max="2000" step="1">

    <label for="bcy">How far down from the top, in millimetres</label>
    <input type="number" id="bcy" name="y" value="40" min="0" max="2000" step="1">

    <label for="level">How much of a QR code can be lost
      <span class="hint">— ignored for a barcode.</span></label>
    <select id="level" name="level">
      <option value="low">low: about 7%</option>
      <option value="medium" selected>medium: about 15%</option>
      <option value="quartile">quartile: about 25%</option>
      <option value="high">high: about 30%</option>
    </select>
  </fieldset>

  <button type="submit">Write the delta</button>
</form>
<p class="hint">
  A barcode has to go on blank paper. Toner goes on top of what is already
  printed, and printing showing through the bars changes their widths — a scanner
  will not read it at all.
</p>

<h2>Stamp a word across a printed sheet</h2>
<p>
  DRAFT, COPY, VOID — corner to corner, on every page. The size works itself out
  from the word, so a short one and a long one both fill the paper.
</p>

<form method="post" action="/watermark" enctype="multipart/form-data">
  <fieldset>
    <legend>The sheet</legend>

    <label for="sheet">The document
      <span class="hint">— a PDF, or a Word or OpenDocument file. Not a picture:
      a scan is written on with <code>onionskin add</code>.</span></label>
    <input type="file" id="sheet" name="sheet"
      accept=".pdf,.docx,.doc,.odt,.rtf" required>

    <label for="text">The word</label>
    <input type="text" id="text" name="text" value="DRAFT" maxlength="60">

    <label for="grey">How light
      <span class="hint">— 75 is the default. Lower is darker.</span></label>
    <input type="number" id="grey" name="grey" value="75" min="0" max="100" step="5">
  </fieldset>

  <button type="submit">Stamp it</button>
</form>
<p class="hint">
  Toner goes on top. This does not sit behind the page&#39;s own printing the way
  a word processor&#39;s watermark does — it goes over it, and where it crosses
  printing the printing wins. Much below 50 and the words underneath stop being
  readable, which is right for a superseded form and wrong for a draft.
</p>

<div class="warn">
  <strong>Print it at 100%.</strong> Put the printed sheet back in the tray, and
  turn <em>Fit to page</em> off — it scales by a few percent and nothing will
  line up. Do one sheet first and hold it against the original.
</div>

<h2>If it refuses</h2>
<p>
  Adding a word in the middle of a paragraph pushes everything after it down the
  page. The delta is then not just your new word but the whole re-flowed
  remainder, at positions that no longer match the sheet in your hand — and
  toner does not come off paper. Onionskin notices, and stops.
</p>
<p>
  To add text without disturbing the layout, put it in a Word text box set to
  <em>Fixed position on page</em> with no text wrapping.
</p>

<footer>
  <p>
    Onionskin never sends your documents anywhere. This page is served from your own machine
    and contains nothing fetched from anywhere else.
  </p>
  <p>
    This page does three things. Onionskin does a great many more — one sheet
    each for everybody on a spreadsheet, sheets of labels, covering something up
    before you hand a document over, fixing a mistake on a page that is already
    printed, checking a whole run came out right. All of them are in the window
    (<code>onionskin-desktop</code>) and on the command line
    (<code>onionskin --help</code>). This page stays small on purpose: it is one
    file with nothing fetched from anywhere, which is what lets it promise it
    never touches the network.
  </p>
</footer>

</body>
</html>
"#;

#[cfg(test)]
mod tests;
