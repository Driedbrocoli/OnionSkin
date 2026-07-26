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
            // Nothing to say: hand the file straight over, which is what
            // somebody who asked for a delta wanted.
            Ok((pdf, said)) if said.is_empty() => respond_file(&mut stream, &pdf, "delta.pdf"),
            Ok((pdf, said)) => {
                let token = set_aside(pdf);
                respond(
                    &mut stream,
                    200,
                    "text/html; charset=utf-8",
                    result_page(&said, &token).as_bytes(),
                )
            }
            Err(message) => respond(
                &mut stream,
                422,
                "text/plain; charset=utf-8",
                message.as_bytes(),
            ),
        },
        // Collecting the delta the page above offered.
        ("GET", path) if path.starts_with("/delta/") => {
            match collect(path.trim_start_matches("/delta/")) {
                Some(pdf) => respond_file(&mut stream, &pdf, "delta.pdf"),
                None => respond(
                    &mut stream,
                    404,
                    "text/plain; charset=utf-8",
                    b"That delta has already been collected. Make it again from the front page.",
                ),
            }
        }
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
static WAITING: std::sync::Mutex<Vec<(String, Vec<u8>)>> = std::sync::Mutex::new(Vec::new());

/// How many deltas may be waiting at once. Enough for somebody with three tabs
/// open, and far short of filling memory with documents nobody came back for.
const MOST_WAITING: usize = 4;

/// Put a finished delta aside and hand back the name to collect it by.
fn set_aside(pdf: Vec<u8>) -> String {
    let token = unique_token();
    let mut waiting = WAITING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    while waiting.len() >= MOST_WAITING {
        waiting.remove(0);
    }
    waiting.push((token.clone(), pdf));
    token
}

/// Collect one, once.
fn collect(token: &str) -> Option<Vec<u8>> {
    let mut waiting = WAITING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let at = waiting.iter().position(|(name, _)| name == token)?;
    Some(waiting.remove(at).1)
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

fn make_delta(request: &Request) -> Result<(Vec<u8>, Vec<String>), String> {
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
    let write = |part: &Part, fallback: &str| -> Result<PathBuf, String> {
        let path = workspace.path.join(safe_name(
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
    let original_path = write(original, "original.pdf")?;
    let edited_path = write(edited, "edited.pdf")?;
    let output = workspace.path.join("delta.pdf");

    let number = |name: &str, fallback: f64| -> f64 {
        field(name)
            .map(|p| p.text())
            .and_then(|t| t.parse::<f64>().ok())
            .filter(|v| v.is_finite())
            .unwrap_or(fallback)
    };
    let options = pipeline::Options {
        dpi: number("dpi", pipeline::DEFAULT_DPI),
        mode: field("mode")
            .and_then(|p| pipeline::Mode::parse(&p.text()))
            .unwrap_or(pipeline::Mode::Raster),
        margin_mm: number("margin", crate::safety::DEFAULT_MARGIN_MM),
        profile: field("profile").map(|p| p.text()).filter(|t| !t.is_empty()),
        ..Default::default()
    };

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
    Ok((pdf, said))
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
fn convert_scan(request: &Request) -> Result<(Vec<u8>, String), String> {
    use crate::letters::{read_with_font, ReadOptions};
    use crate::office::{self, Format, Layout};
    use crate::scan::{register, ScanOptions};

    let parts = parse_multipart(&request.content_type, &request.body)?;
    let field = |name: &str| parts.iter().find(|p| p.name == name);

    let Some(scan) = field("scan").filter(|p| !p.data.is_empty()) else {
        return Err("Choose a scan to read: a PNG, JPEG, TIFF or BMP.".into());
    };
    let Some(font_part) = field("font").filter(|p| !p.data.is_empty()) else {
        return Err(
            "Choose the font the page was set in.\n\nWithout it Onionskin can see \
             where the ink is but not what it says, and there would be nothing to \
             write into the document. Any .ttf or .otf file will do — on Windows \
             they are in C:\\Windows\\Fonts, on macOS in /Library/Fonts, on Linux \
             in /usr/share/fonts."
                .into(),
        );
    };

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

    let image = image::load_from_memory(&scan.data)
        .map_err(|e| format!("That does not look like an image Onionskin can read: {e}"))?;
    let registration = register(&image, ScanOptions::new(page)).map_err(|e| e.to_string())?;
    let gray = image.to_luma8();

    // The font has to be a file on disk, because that is what the loader takes
    // — it memory-maps the programme rather than copying it about.
    let workspace = Workspace::new(false).map_err(|e| e.to_string())?;
    let font_path = workspace.path.join(safe_name(
        font_part.filename.as_deref().unwrap_or("font.ttf"),
        "font.ttf",
    ));
    std::fs::write(&font_path, &font_part.data).map_err(|e| e.to_string())?;
    let font = crate::font::EmbeddedFont::load(&font_path).map_err(|e| e.to_string())?;

    let text = read_with_font(&gray, &registration, &ReadOptions::default(), &font, None)
        .map_err(|e| e.to_string())?;
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
fn page() -> String {
    format!("{HEAD}{PAGE_BODY}")
}

/// What a finished run had to say, and a way to fetch the delta.
///
/// A browser is handed one thing per request, so a run with something worth
/// saying says it here and offers the file second. Nothing is lost by that: the
/// warnings are the reason a person would not print the delta at all, and a
/// file that arrives before them arrives too late to be worth reading.
fn result_page(said: &[String], token: &str) -> String {
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

    format!(
        "{HEAD}\n<h1>The delta is ready</h1>\n\
         <p class=\"lede\">Worth reading before you print it.</p>\n\
         {lines}\n\
         <p><a class=\"get\" href=\"/delta/{token}\" download=\"delta.pdf\">\
         Download delta.pdf</a></p>\n\
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
         <footer>Onionskin never uses the network. Everything here happened on \
         this machine.</footer>\n\
         </body>\n</html>\n"
    )
}

/// The five characters HTML cannot take literally. A document called
/// `Smith &amp; Sons <draft>` would otherwise close a tag nobody opened.
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
  </fieldset>

  <fieldset>
    <legend>Settings</legend>
    <div class="row">
      <div>
        <label for="mode">Delta</label>
        <select id="mode" name="mode">
          <option value="raster">Raster — exactly the new pixels</option>
          <option value="vector">Vector — sharper, clips to boxes</option>
        </select>
      </div>
      <div>
        <label for="dpi">Resolution</label>
        <input type="number" id="dpi" name="dpi" value="400" min="50" max="1200" step="50">
      </div>
      <div>
        <label for="margin">Edge margin (mm)</label>
        <input type="number" id="margin" name="margin" value="5" min="0" max="40" step="0.5">
      </div>
      <div>
        <label for="profile">Printer profile
          <span class="hint">— optional</span></label>
        <input type="text" id="profile" name="profile" placeholder="office">
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
      <span class="hint">— .png, .jpg, .tiff, .bmp</span></label>
    <input type="file" id="scan" name="scan" accept="image/*" required>

    <label for="font">The font the page was set in
      <span class="hint">— .ttf or .otf. Without it, Onionskin can see where
      the ink is but not what it says.</span></label>
    <input type="file" id="font" name="font" accept=".ttf,.otf,.ttc" required>

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
        <label for="page">Paper</label>
        <input type="text" id="page" name="page" value="a4" placeholder="a4">
      </div>
    </div>
  </fieldset>

  <button type="submit">Read it</button>
</form>

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
    Onionskin never uses the network. This page is served from your own machine
    and contains nothing fetched from anywhere else.
  </p>
  <p>
    For typing straight onto a page, filling a scanned form, or reading the
    letters off a scan, use the command line: <code>onionskin --help</code>.
  </p>
</footer>

</body>
</html>
"#;

#[cfg(test)]
mod tests;
