//! Talking to a printer directly — printing to it, and scanning from it.
//!
//! Two protocols, both spoken here in Rust with nothing underneath them but a
//! TCP socket:
//!
//! * **IPP**, the Internet Printing Protocol, is how every network printer made
//!   this century accepts a job, and how CUPS talks to the ones plugged in by
//!   USB. Onionskin uses it to send a delta straight to the paper.
//! * **eSCL**, the protocol behind AirScan, is how the same machines hand back
//!   a scan. Onionskin uses it to fetch the sheet without any scanning software
//!   installed at all.
//!
//! Between them they turn a multifunction printer into both halves of the job:
//! scan the sheet, work out what to add, print it back onto the same paper.
//!
//! # About the network
//!
//! Onionskin's promise is that it never phones home — no telemetry, no update
//! check, nothing about your documents leaving your machine. That promise is
//! unchanged and worth being exact about. What happens here is that when *you*
//! name a printer, Onionskin talks to *that printer*, on your own network, and
//! to nothing else. There is no discovery beacon, no directory service, no
//! address anywhere in this file. If you never name a printer, not one packet
//! leaves the machine.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::time::Duration;

/// How long to wait on a printer before giving up.
///
/// Generous. A printer waking from sleep takes its time, and a spurious
/// timeout looks exactly like a printer that is switched off — which sends
/// somebody to check the cable for no reason.
const TIMEOUT: Duration = Duration::from_secs(60);

/// A scan can take much longer: the lamp has to warm up and the carriage has to
/// cross the glass before a single byte comes back.
const SCAN_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, thiserror::Error)]
pub enum PrinterError {
    #[error("{0}")]
    Address(String),
    #[error("could not reach {host}: {source}\n    Check the printer is switched on and on this network.")]
    Unreachable {
        host: String,
        source: std::io::Error,
    },
    #[error("{0}")]
    Protocol(String),
    #[error("the printer refused the job: {0}")]
    Refused(String),
    #[error("could not read {path}: {source}")]
    Io {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
}

// ---------------------------------------------------------------------------
// A very small HTTP client
// ---------------------------------------------------------------------------

/// An address broken into the pieces a request needs.
#[derive(Debug, Clone, PartialEq)]
pub struct Address {
    pub host: String,
    pub port: u16,
    pub path: String,
    /// The scheme as written, kept so an error can quote it back.
    pub scheme: String,
}

impl Address {
    /// Parse `ipp://printer.local/ipp/print`, or anything close enough to it.
    ///
    /// Deliberately forgiving about the scheme: `ipp://` and `http://` mean the
    /// same thing on the wire, `ipps://` is the encrypted one, and somebody who
    /// types a bare address means the obvious thing.
    pub fn parse(
        text: &str,
        default_port: u16,
        default_path: &str,
    ) -> Result<Address, PrinterError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(PrinterError::Address("no printer address given".into()));
        }
        let (scheme, rest) = match text.split_once("://") {
            Some((scheme, rest)) => (scheme.to_ascii_lowercase(), rest),
            None => ("ipp".to_string(), text),
        };
        if scheme == "ipps" || scheme == "https" {
            return Err(PrinterError::Address(format!(
                "{scheme}:// is the encrypted form, which Onionskin does not speak \
                 yet.\n    Most printers accept ipp:// on port 631 as well — try \
                 that, or print the delta from your usual print dialogue."
            )));
        }

        let (authority, path) = match rest.find('/') {
            Some(at) => (&rest[..at], &rest[at..]),
            None => (rest, default_path),
        };
        if authority.is_empty() {
            return Err(PrinterError::Address(format!(
                "'{text}' has no printer address in it"
            )));
        }

        // An IPv6 address is written in brackets, and its colons are not the
        // colon that introduces a port.
        let (host, port) = if let Some(close) = authority.strip_prefix('[') {
            match close.split_once(']') {
                Some((inside, after)) => (
                    inside.to_string(),
                    port_of(after.strip_prefix(':'), default_port, text)?,
                ),
                None => {
                    return Err(PrinterError::Address(format!(
                        "'{text}' opens a bracket for an IPv6 address and never closes it"
                    )))
                }
            }
        } else {
            match authority.rsplit_once(':') {
                Some((host, port)) => (host.to_string(), port_of(Some(port), default_port, text)?),
                None => (authority.to_string(), default_port),
            }
        };

        Ok(Address {
            host,
            port,
            path: if path.is_empty() {
                default_path.to_string()
            } else {
                path.to_string()
            },
            scheme,
        })
    }

    /// The `ipp://` form, which is what an IPP request has to carry inside it.
    pub fn ipp_uri(&self) -> String {
        let host = if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        if self.port == 631 {
            format!("ipp://{host}{}", self.path)
        } else {
            format!("ipp://{host}:{}{}", self.port, self.path)
        }
    }

    fn connect(&self, timeout: Duration) -> Result<TcpStream, PrinterError> {
        use std::net::ToSocketAddrs;
        let target = format!("{}:{}", self.host, self.port);
        let mut last: Option<std::io::Error> = None;

        let addresses = target
            .to_socket_addrs()
            .map_err(|source| PrinterError::Unreachable {
                host: self.host.clone(),
                source,
            })?;
        for address in addresses {
            match TcpStream::connect_timeout(&address, timeout) {
                Ok(stream) => {
                    let _ = stream.set_read_timeout(Some(timeout));
                    let _ = stream.set_write_timeout(Some(timeout));
                    return Ok(stream);
                }
                Err(e) => last = Some(e),
            }
        }
        Err(PrinterError::Unreachable {
            host: self.host.clone(),
            source: last.unwrap_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotFound, "no address for that name")
            }),
        })
    }
}

fn port_of(text: Option<&str>, default_port: u16, whole: &str) -> Result<u16, PrinterError> {
    match text {
        None | Some("") => Ok(default_port),
        Some(digits) => digits.parse().map_err(|_| {
            PrinterError::Address(format!("'{digits}' is not a port number, in '{whole}'"))
        }),
    }
}

/// What came back.
#[derive(Debug)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// One HTTP request, written straight onto a socket.
///
/// No keep-alive, no chunked request bodies, no redirects. A printer is one
/// machine on the far side of a cable answering one question, and everything a
/// general client does for the open web is weight without purpose here.
fn http(
    address: &Address,
    method: &str,
    path: &str,
    content_type: &str,
    body: &[u8],
    timeout: Duration,
) -> Result<Response, PrinterError> {
    let mut stream = address.connect(timeout)?;
    let host = if address.host.contains(':') {
        format!("[{}]:{}", address.host, address.port)
    } else {
        format!("{}:{}", address.host, address.port)
    };

    let mut request = format!(
        "{method} {path} HTTP/1.1\r\n\
         Host: {host}\r\n\
         User-Agent: Onionskin\r\n\
         Connection: close\r\n"
    );
    if !content_type.is_empty() {
        request.push_str(&format!("Content-Type: {content_type}\r\n"));
    }
    request.push_str(&format!("Content-Length: {}\r\n\r\n", body.len()));

    stream
        .write_all(request.as_bytes())
        .and_then(|()| stream.write_all(body))
        .and_then(|()| stream.flush())
        .map_err(|source| PrinterError::Unreachable {
            host: address.host.clone(),
            source,
        })?;

    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .map_err(|source| PrinterError::Unreachable {
            host: address.host.clone(),
            source,
        })?;

    parse_response(&raw)
}

fn parse_response(raw: &[u8]) -> Result<Response, PrinterError> {
    let split = find(raw, b"\r\n\r\n")
        .ok_or_else(|| PrinterError::Protocol("the reply had no headers in it".into()))?;
    let head = String::from_utf8_lossy(&raw[..split]);
    let mut lines = head.lines();

    let status_line = lines
        .next()
        .ok_or_else(|| PrinterError::Protocol("the reply was empty".into()))?;
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .ok_or_else(|| PrinterError::Protocol(format!("'{status_line}' is not an HTTP reply")))?;

    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_string(), value.trim().to_string()))
        .collect();

    let mut body = raw[split + 4..].to_vec();
    // Chunked replies are ordinary from printers, which often cannot say how
    // long an answer will be until they have finished composing it.
    if headers
        .iter()
        .any(|(k, v)| k.eq_ignore_ascii_case("transfer-encoding") && v.contains("chunked"))
    {
        body = dechunk(&body)?;
    }

    Ok(Response {
        status,
        headers,
        body,
    })
}

/// Undo `Transfer-Encoding: chunked`.
fn dechunk(body: &[u8]) -> Result<Vec<u8>, PrinterError> {
    let mut out = Vec::new();
    let mut at = 0usize;
    loop {
        let Some(end) = find(&body[at..], b"\r\n") else {
            break;
        };
        let header = String::from_utf8_lossy(&body[at..at + end]);
        // A chunk header may carry extensions after a semicolon.
        let size_text = header.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_text, 16)
            .map_err(|_| PrinterError::Protocol(format!("'{size_text}' is not a chunk length")))?;
        at += end + 2;
        if size == 0 {
            break;
        }
        if at + size > body.len() {
            return Err(PrinterError::Protocol(
                "the reply ended in the middle of a chunk".into(),
            ));
        }
        out.extend_from_slice(&body[at..at + size]);
        at += size + 2; // the CRLF that follows every chunk
    }
    Ok(out)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    (0..=haystack.len() - needle.len()).find(|&i| &haystack[i..i + needle.len()] == needle)
}

// ---------------------------------------------------------------------------
// IPP — printing
// ---------------------------------------------------------------------------

/// The IPP operations Onionskin needs.
mod operation {
    pub const PRINT_JOB: u16 = 0x0002;
    pub const GET_PRINTER_ATTRIBUTES: u16 = 0x000B;
    pub const CUPS_GET_PRINTERS: u16 = 0x4002;
}

/// IPP value tags, from RFC 8010.
mod tag {
    pub const OPERATION_ATTRIBUTES: u8 = 0x01;
    pub const JOB_ATTRIBUTES: u8 = 0x02;
    pub const END_OF_ATTRIBUTES: u8 = 0x03;
    pub const PRINTER_ATTRIBUTES: u8 = 0x04;

    pub const INTEGER: u8 = 0x21;
    pub const BOOLEAN: u8 = 0x22;
    pub const ENUM: u8 = 0x23;
    pub const TEXT: u8 = 0x41;
    pub const NAME: u8 = 0x42;
    pub const KEYWORD: u8 = 0x44;
    pub const URI: u8 = 0x45;
    pub const CHARSET: u8 = 0x47;
    pub const LANGUAGE: u8 = 0x48;
    pub const MIME_TYPE: u8 = 0x49;
}

/// An IPP request being built up, byte by byte.
struct IppRequest {
    bytes: Vec<u8>,
}

impl IppRequest {
    /// IPP 1.1: understood by every printer that speaks IPP at all, where 2.0
    /// is not. There is nothing here that needs the newer version.
    fn new(operation: u16, request_id: u32) -> IppRequest {
        let mut bytes = vec![0x01, 0x01];
        bytes.extend_from_slice(&operation.to_be_bytes());
        bytes.extend_from_slice(&request_id.to_be_bytes());
        bytes.push(tag::OPERATION_ATTRIBUTES);
        IppRequest { bytes }
    }

    fn group(&mut self, group: u8) -> &mut Self {
        self.bytes.push(group);
        self
    }

    fn attribute(&mut self, value_tag: u8, name: &str, value: &[u8]) -> &mut Self {
        self.bytes.push(value_tag);
        self.bytes
            .extend_from_slice(&(name.len() as u16).to_be_bytes());
        self.bytes.extend_from_slice(name.as_bytes());
        self.bytes
            .extend_from_slice(&(value.len() as u16).to_be_bytes());
        self.bytes.extend_from_slice(value);
        self
    }

    fn text(&mut self, value_tag: u8, name: &str, value: &str) -> &mut Self {
        self.attribute(value_tag, name, value.as_bytes())
    }

    fn integer(&mut self, name: &str, value: i32) -> &mut Self {
        self.attribute(tag::INTEGER, name, &value.to_be_bytes())
    }

    /// Another value for the attribute just written.
    ///
    /// A repeated attribute is written as an entry with an empty name, which is
    /// how IPP says "and also this" without repeating the key.
    fn also(&mut self, value_tag: u8, value: &[u8]) -> &mut Self {
        self.attribute(value_tag, "", value)
    }

    fn finish(mut self, document: &[u8]) -> Vec<u8> {
        self.bytes.push(tag::END_OF_ATTRIBUTES);
        self.bytes.extend_from_slice(document);
        self.bytes
    }
}

/// The attributes IPP requires at the head of every request, in this order.
fn preamble(request: &mut IppRequest, uri: &str, user: &str) {
    request
        .text(tag::CHARSET, "attributes-charset", "utf-8")
        .text(tag::LANGUAGE, "attributes-natural-language", "en")
        .text(tag::URI, "printer-uri", uri)
        .text(tag::NAME, "requesting-user-name", user);
}

/// Who to say the job is from.
fn whoami() -> String {
    for key in ["USER", "USERNAME", "LOGNAME"] {
        if let Ok(name) = std::env::var(key) {
            if !name.is_empty() {
                return name;
            }
        }
    }
    "onionskin".to_string()
}

/// A parsed IPP reply, flattened to the attributes that matter.
#[derive(Debug, Default)]
pub struct IppReply {
    pub status: u16,
    /// Every attribute seen, in order, as `(group, name, value)`.
    pub attributes: Vec<(u8, String, IppValue)>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum IppValue {
    Text(String),
    Integer(i32),
    Boolean(bool),
    Other(Vec<u8>),
}

impl IppValue {
    pub fn as_text(&self) -> Option<&str> {
        match self {
            IppValue::Text(text) => Some(text),
            _ => None,
        }
    }
    pub fn as_integer(&self) -> Option<i32> {
        match self {
            IppValue::Integer(value) => Some(*value),
            _ => None,
        }
    }
}

impl IppReply {
    /// Anything below 0x0100 is a success; the rest are warnings and errors.
    pub fn succeeded(&self) -> bool {
        self.status < 0x0100
    }

    pub fn get(&self, name: &str) -> Option<&IppValue> {
        self.attributes
            .iter()
            .find(|(_, key, _)| key == name)
            .map(|(_, _, value)| value)
    }

    /// What went wrong, in the printer's own words where it offered any.
    pub fn complaint(&self) -> String {
        let reason = self
            .get("status-message")
            .and_then(|v| v.as_text())
            .map(str::to_string)
            .unwrap_or_else(|| status_name(self.status).to_string());
        format!("{reason} (IPP status 0x{:04x})", self.status)
    }
}

/// The IPP statuses worth naming, because their numbers say nothing.
fn status_name(status: u16) -> &'static str {
    match status {
        0x0000 => "successful",
        0x0400 => "the request was malformed",
        0x0401 => "the printer would not allow it",
        0x0402 => "the printer does not support that operation",
        0x0403 => "the printer does not support that version of IPP",
        0x0405 => "there is no printer at that address",
        0x040A => "the printer does not understand that document format",
        0x040B => "the printer refused the attributes sent with the job",
        0x0501 => "something went wrong inside the printer",
        0x0502 => "the printer does not support that operation",
        0x0503 => "the printer is not accepting jobs at the moment",
        0x0507 => "the printer is busy",
        _ => "the printer refused it",
    }
}

fn parse_ipp(body: &[u8]) -> Result<IppReply, PrinterError> {
    if body.len() < 8 {
        return Err(PrinterError::Protocol(
            "the printer's reply was too short to be IPP".into(),
        ));
    }
    let status = u16::from_be_bytes([body[2], body[3]]);
    let mut reply = IppReply {
        status,
        attributes: Vec::new(),
    };

    let mut at = 8usize;
    let mut group = 0u8;
    let mut last_name = String::new();

    while at < body.len() {
        let value_tag = body[at];
        at += 1;
        // A tag below 0x10 opens a new group rather than carrying a value.
        if value_tag < 0x10 {
            if value_tag == tag::END_OF_ATTRIBUTES {
                break;
            }
            group = value_tag;
            continue;
        }
        if at + 2 > body.len() {
            break;
        }
        let name_len = u16::from_be_bytes([body[at], body[at + 1]]) as usize;
        at += 2;
        if at + name_len > body.len() {
            break;
        }
        let name = String::from_utf8_lossy(&body[at..at + name_len]).to_string();
        at += name_len;

        if at + 2 > body.len() {
            break;
        }
        let value_len = u16::from_be_bytes([body[at], body[at + 1]]) as usize;
        at += 2;
        if at + value_len > body.len() {
            break;
        }
        let raw = &body[at..at + value_len];
        at += value_len;

        // An empty name means another value for the attribute before it.
        let name = if name.is_empty() {
            last_name.clone()
        } else {
            last_name = name.clone();
            name
        };

        let value = match value_tag {
            tag::INTEGER | tag::ENUM => IppValue::Integer(match raw.len() {
                4 => i32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]),
                _ => 0,
            }),
            tag::BOOLEAN => IppValue::Boolean(raw.first().copied().unwrap_or(0) != 0),
            tag::TEXT
            | tag::NAME
            | tag::KEYWORD
            | tag::URI
            | tag::CHARSET
            | tag::LANGUAGE
            | tag::MIME_TYPE => IppValue::Text(String::from_utf8_lossy(raw).to_string()),
            _ => IppValue::Other(raw.to_vec()),
        };
        reply.attributes.push((group, name, value));
    }
    Ok(reply)
}

/// A printer this machine can send paper to.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Printer {
    pub name: String,
    pub uri: String,
    /// What it calls itself: make and model, usually.
    pub model: String,
    /// Where somebody said it is — "second floor", "by the kitchen".
    pub location: String,
    /// Whether it says it is ready.
    pub state: String,
}

/// Ask a print server which printers it has.
///
/// Aimed at CUPS, which is what macOS and every Linux desktop run, and which a
/// USB printer appears through. A printer asked this directly will not
/// understand the operation and says so, which is a fair answer rather than a
/// failure.
pub fn printers(server: &str) -> Result<Vec<Printer>, PrinterError> {
    let address = Address::parse(server, 631, "/")?;
    let mut request = IppRequest::new(operation::CUPS_GET_PRINTERS, 1);
    request
        .text(tag::CHARSET, "attributes-charset", "utf-8")
        .text(tag::LANGUAGE, "attributes-natural-language", "en")
        .text(tag::KEYWORD, "requested-attributes", "printer-name")
        .also(tag::KEYWORD, b"printer-uri-supported")
        .also(tag::KEYWORD, b"printer-make-and-model")
        .also(tag::KEYWORD, b"printer-location")
        .also(tag::KEYWORD, b"printer-state");
    let body = request.finish(&[]);

    let response = http(&address, "POST", "/", "application/ipp", &body, TIMEOUT)?;
    if response.status != 200 {
        return Err(PrinterError::Protocol(format!(
            "the print server answered HTTP {} rather than a printer list",
            response.status
        )));
    }
    let reply = parse_ipp(&response.body)?;
    if !reply.succeeded() {
        return Err(PrinterError::Refused(reply.complaint()));
    }

    // Each printer is one printer-attributes group, so a repeat of the first
    // key starts a new one.
    let mut found: Vec<Printer> = Vec::new();
    let mut current: Option<Printer> = None;
    for (group, name, value) in &reply.attributes {
        if *group != tag::PRINTER_ATTRIBUTES {
            continue;
        }
        let text = value.as_text().unwrap_or("").to_string();
        match name.as_str() {
            "printer-name" => {
                if let Some(printer) = current.take() {
                    found.push(printer);
                }
                current = Some(Printer {
                    name: text,
                    uri: String::new(),
                    model: String::new(),
                    location: String::new(),
                    state: String::new(),
                });
            }
            "printer-uri-supported" => {
                if let Some(printer) = current.as_mut() {
                    if printer.uri.is_empty() {
                        printer.uri = text;
                    }
                }
            }
            "printer-make-and-model" => {
                if let Some(printer) = current.as_mut() {
                    printer.model = text;
                }
            }
            "printer-location" => {
                if let Some(printer) = current.as_mut() {
                    printer.location = text;
                }
            }
            "printer-state" => {
                if let Some(printer) = current.as_mut() {
                    printer.state = match value.as_integer() {
                        Some(3) => "idle".into(),
                        Some(4) => "printing".into(),
                        Some(5) => "stopped".into(),
                        _ => String::new(),
                    };
                }
            }
            _ => {}
        }
    }
    if let Some(printer) = current {
        found.push(printer);
    }
    Ok(found)
}

/// What a printer says about itself.
pub fn describe(printer_uri: &str) -> Result<Vec<(String, String)>, PrinterError> {
    let address = Address::parse(printer_uri, 631, "/ipp/print")?;
    let mut request = IppRequest::new(operation::GET_PRINTER_ATTRIBUTES, 1);
    preamble(&mut request, &address.ipp_uri(), &whoami());
    request
        .text(
            tag::KEYWORD,
            "requested-attributes",
            "printer-make-and-model",
        )
        .also(tag::KEYWORD, b"printer-state")
        .also(tag::KEYWORD, b"printer-state-message")
        .also(tag::KEYWORD, b"document-format-supported")
        .also(tag::KEYWORD, b"media-default")
        .also(tag::KEYWORD, b"printer-resolution-default");
    let body = request.finish(&[]);

    let response = http(
        &address,
        "POST",
        &address.path,
        "application/ipp",
        &body,
        TIMEOUT,
    )?;
    let reply = parse_ipp(&response.body)?;
    if !reply.succeeded() {
        return Err(PrinterError::Refused(reply.complaint()));
    }
    Ok(reply
        .attributes
        .iter()
        .filter(|(group, _, _)| *group == tag::PRINTER_ATTRIBUTES)
        .map(|(_, name, value)| {
            let text = match value {
                IppValue::Text(text) => text.clone(),
                IppValue::Integer(number) => number.to_string(),
                IppValue::Boolean(yes) => yes.to_string(),
                IppValue::Other(bytes) => format!("{} bytes", bytes.len()),
            };
            (name.clone(), text)
        })
        .collect())
}

/// How to print it.
#[derive(Debug, Clone)]
pub struct PrintOptions {
    pub copies: u32,
    /// What to call the job in the queue.
    pub job_name: String,
    /// The media size by its IPP name, such as `iso_a4_210x297mm`. `None`
    /// leaves the printer on its own default.
    pub media: Option<String>,
    /// Double-sided. Left alone unless asked, because a delta printed on the
    /// back of the sheet it was meant for is a wasted sheet.
    pub two_sided: bool,
}

impl Default for PrintOptions {
    fn default() -> Self {
        Self {
            copies: 1,
            job_name: "Onionskin delta".to_string(),
            media: None,
            two_sided: false,
        }
    }
}

/// Send a PDF to a printer, and return the job number it was given.
///
/// The scaling attributes are the point of this being here at all rather than
/// leaving it to a print dialogue. A delta has to go on the paper at exactly
/// 100%, and every print dialogue in the world defaults to fitting the page —
/// which scales by a percent or two and puts every word in the wrong place.
/// Sent this way, the instruction not to scale travels with the job.
pub fn print_file(
    printer_uri: &str,
    path: &Path,
    options: &PrintOptions,
) -> Result<i32, PrinterError> {
    let document = std::fs::read(path).map_err(|source| PrinterError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    print_bytes(printer_uri, &document, options)
}

/// The same, for a delta that is already in memory.
pub fn print_bytes(
    printer_uri: &str,
    document: &[u8],
    options: &PrintOptions,
) -> Result<i32, PrinterError> {
    if document.is_empty() {
        return Err(PrinterError::Protocol("there is nothing to print".into()));
    }
    if options.copies == 0 || options.copies > 999 {
        return Err(PrinterError::Protocol(format!(
            "{} copies is not a number of copies",
            options.copies
        )));
    }
    let address = Address::parse(printer_uri, 631, "/ipp/print")?;
    let uri = address.ipp_uri();

    let mut request = IppRequest::new(operation::PRINT_JOB, 1);
    preamble(&mut request, &uri, &whoami());
    request.text(tag::NAME, "job-name", &options.job_name).text(
        tag::MIME_TYPE,
        "document-format",
        "application/pdf",
    );

    request.group(tag::JOB_ATTRIBUTES);
    request.integer("copies", options.copies as i32);
    // `none` is the whole reason for printing this way: it tells the printer to
    // put the page on the paper at its true size and not to helpfully shrink it
    // to fit inside the printable area.
    request.text(tag::KEYWORD, "print-scaling", "none");
    request.text(
        tag::KEYWORD,
        "sides",
        if options.two_sided {
            "two-sided-long-edge"
        } else {
            "one-sided"
        },
    );
    if let Some(media) = &options.media {
        request.text(tag::KEYWORD, "media", media);
    }

    let body = request.finish(document);
    let response = http(
        &address,
        "POST",
        &address.path,
        "application/ipp",
        &body,
        TIMEOUT,
    )?;

    if response.status == 426 || response.status == upgrade_required() {
        return Err(PrinterError::Refused(
            "the printer wants an encrypted connection, which Onionskin does not \
             speak yet. Print the delta from your usual print dialogue instead — \
             at 100%, with 'Fit to page' off."
                .into(),
        ));
    }
    if response.status != 200 {
        return Err(PrinterError::Protocol(format!(
            "the printer answered HTTP {} rather than accepting the job",
            response.status
        )));
    }

    let reply = parse_ipp(&response.body)?;
    if !reply.succeeded() {
        return Err(PrinterError::Refused(reply.complaint()));
    }
    Ok(reply
        .get("job-id")
        .and_then(|v| v.as_integer())
        .unwrap_or(0))
}

/// HTTP 426, spelled out so the comparison above reads as what it means.
fn upgrade_required() -> u16 {
    426
}

// ---------------------------------------------------------------------------
// eSCL — scanning
// ---------------------------------------------------------------------------

/// What to ask the scanner in a multifunction printer for.
#[derive(Debug, Clone)]
pub struct ScanRequest {
    /// Dots per inch. 300 is plenty: the sheet's outline is a centimetre-scale
    /// feature, and a finer scan costs time and memory for no better fix.
    pub resolution: u32,
    pub colour: bool,
    /// The sheet on the glass, or the one in the document feeder.
    pub feeder: bool,
    /// The area to scan, in millimetres. The whole platen if not given.
    pub area_mm: Option<(f64, f64)>,
}

impl Default for ScanRequest {
    fn default() -> Self {
        Self {
            resolution: 300,
            colour: false,
            feeder: false,
            area_mm: None,
        }
    }
}

/// eSCL measures the glass in three-hundredths of an inch, whatever resolution
/// the scan itself is at.
const ESCL_UNITS_PER_INCH: f64 = 300.0;

fn to_escl_units(mm: f64) -> i64 {
    (mm / crate::geometry::MM_PER_INCH * ESCL_UNITS_PER_INCH).round() as i64
}

/// The request document eSCL expects.
///
/// Written out by hand rather than through an XML library. It is forty lines of
/// fixed text with six numbers in it, the numbers are all integers Onionskin
/// produced itself, and a dependency that can serialise arbitrary documents
/// would earn its place only if there were arbitrary documents to serialise.
pub fn scan_settings_xml(request: &ScanRequest) -> String {
    let (width_mm, height_mm) = request.area_mm.unwrap_or((215.9, 355.6));
    let source = if request.feeder { "Feeder" } else { "Platen" };
    let colour = if request.colour {
        "RGB24"
    } else {
        "Grayscale8"
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<scan:ScanSettings xmlns:pwg="http://www.pwg.org/schemas/2010/12/sm" xmlns:scan="http://schemas.hp.com/imaging/escl/2011/05/03">
  <pwg:Version>2.6</pwg:Version>
  <scan:Intent>Document</scan:Intent>
  <pwg:ScanRegions pwg:MustHonor="false">
    <pwg:ScanRegion>
      <pwg:XOffset>0</pwg:XOffset>
      <pwg:YOffset>0</pwg:YOffset>
      <pwg:Width>{width}</pwg:Width>
      <pwg:Height>{height}</pwg:Height>
      <pwg:ContentRegionUnits>escl:ThreeHundredthsOfInches</pwg:ContentRegionUnits>
    </pwg:ScanRegion>
  </pwg:ScanRegions>
  <pwg:InputSource>{source}</pwg:InputSource>
  <scan:ColorMode>{colour}</scan:ColorMode>
  <scan:XResolution>{dpi}</scan:XResolution>
  <scan:YResolution>{dpi}</scan:YResolution>
  <pwg:DocumentFormat>image/jpeg</pwg:DocumentFormat>
</scan:ScanSettings>
"#,
        width = to_escl_units(width_mm),
        height = to_escl_units(height_mm),
        source = source,
        colour = colour,
        dpi = request.resolution,
    )
}

/// Scan a sheet from a multifunction printer, and write it to `out`.
///
/// The sheet comes back as an image, not as a document: Onionskin has to find
/// the paper's outline in it to know how big the page is and how far it is
/// turned, and a scanner that has already cropped and straightened the picture
/// has thrown that away. The request above therefore asks for the whole platen
/// and no automatic anything.
pub fn scan_to(
    scanner_uri: &str,
    request: &ScanRequest,
    out: &Path,
) -> Result<std::path::PathBuf, PrinterError> {
    if !(50..=2400).contains(&request.resolution) {
        return Err(PrinterError::Protocol(format!(
            "{} dpi is outside what a scanner will do (50 to 2400)",
            request.resolution
        )));
    }
    let address = Address::parse(scanner_uri, 80, "/eSCL")?;
    let base = address.path.trim_end_matches('/').to_string();

    let settings = scan_settings_xml(request);
    let response = http(
        &address,
        "POST",
        &format!("{base}/ScanJobs"),
        "text/xml",
        settings.as_bytes(),
        TIMEOUT,
    )?;

    // 201 Created, with the job's address in the Location header.
    if response.status != 201 {
        return Err(PrinterError::Refused(match response.status {
            404 => format!(
                "there is no scanner at {scanner_uri}.\n    Most printers put it at \
                 /eSCL — try 'http://{}:{}/eSCL'.",
                address.host, address.port
            ),
            409 => "the scanner is busy with another job.".to_string(),
            503 => "the scanner is not ready — check for a paper jam or an open lid.".to_string(),
            other => format!("the scanner answered HTTP {other} to the scan request"),
        }));
    }
    let location = response
        .header("Location")
        .ok_or_else(|| {
            PrinterError::Protocol(
                "the scanner accepted the job but did not say where to collect it".into(),
            )
        })?
        .to_string();

    // The Location may be absolute or relative; only its path is needed.
    let job_path = match location.split_once("://") {
        Some((_, rest)) => match rest.find('/') {
            Some(at) => rest[at..].to_string(),
            None => "/".to_string(),
        },
        None => location,
    };

    // Collecting the page blocks until the carriage has crossed the glass.
    let page = http(
        &address,
        "GET",
        &format!("{}/NextDocument", job_path.trim_end_matches('/')),
        "",
        &[],
        SCAN_TIMEOUT,
    )?;
    if page.status != 200 {
        return Err(PrinterError::Refused(format!(
            "the scan was accepted but came back HTTP {} when collected",
            page.status
        )));
    }
    if page.body.is_empty() {
        return Err(PrinterError::Protocol(
            "the scanner returned an empty page. Check there is a sheet on the glass.".into(),
        ));
    }

    if let Some(parent) = out.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|source| PrinterError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
    }
    std::fs::write(out, &page.body).map_err(|source| PrinterError::Io {
        path: out.to_path_buf(),
        source,
    })?;
    Ok(out.to_path_buf())
}

/// Does this address answer as a scanner?
pub fn scanner_present(scanner_uri: &str) -> bool {
    let Ok(address) = Address::parse(scanner_uri, 80, "/eSCL") else {
        return false;
    };
    let base = address.path.trim_end_matches('/').to_string();
    http(
        &address,
        "GET",
        &format!("{base}/ScannerCapabilities"),
        "",
        &[],
        Duration::from_secs(5),
    )
    .map(|response| response.status == 200)
    .unwrap_or(false)
}

/// What a scanner says it can do, as the handful of facts worth acting on.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize)]
pub struct ScannerCapabilities {
    pub make_and_model: String,
    /// Resolutions it offers, in dots per inch.
    pub resolutions: Vec<u32>,
    pub has_platen: bool,
    pub has_feeder: bool,
}

/// Pull one element's text out of an XML document.
///
/// Enough XML for this job and no more: eSCL replies are machine-written, flat,
/// and use a fixed set of element names. Everything read here is a number or a
/// short token, and the only thing that could go wrong with a fuller parser is
/// that it would be a dependency.
fn xml_values(xml: &str, local_name: &str) -> Vec<String> {
    let mut found = Vec::new();
    let bytes = xml.as_bytes();
    let mut at = 0usize;

    while let Some(open) = xml[at..].find('<') {
        let tag_start = at + open + 1;
        if tag_start >= xml.len() {
            break;
        }
        // A closing tag, a declaration or a comment opens nothing.
        if matches!(bytes[tag_start], b'/' | b'?' | b'!') {
            at = tag_start;
            continue;
        }
        let Some(close) = xml[tag_start..].find('>') else {
            break;
        };
        let tag = &xml[tag_start..tag_start + close];
        let text_start = tag_start + close + 1;

        // The name is what comes before any attributes, with the namespace
        // prefix taken off. Matching on the whole tag instead would miss
        // `<scan:XResolution>` and — worse — find `<MaxXResolution>`, which
        // reports a resolution no scan will ever be taken at.
        let name = tag.split_whitespace().next().unwrap_or(tag);
        let name = name.rsplit(':').next().unwrap_or(name);

        if name == local_name && !tag.ends_with('/') {
            // The text runs to the next tag, whatever that tag turns out to be
            // called. Looking for a literal `</name>` would miss the closing
            // tag whenever it carries a namespace prefix — which, in an eSCL
            // reply, is always.
            let text = match xml[text_start..].find('<') {
                Some(end) => &xml[text_start..text_start + end],
                None => &xml[text_start..],
            };
            found.push(text.trim().to_string());
        }
        at = text_start;
    }
    found
}

pub fn capabilities(scanner_uri: &str) -> Result<ScannerCapabilities, PrinterError> {
    let address = Address::parse(scanner_uri, 80, "/eSCL")?;
    let base = address.path.trim_end_matches('/').to_string();
    let response = http(
        &address,
        "GET",
        &format!("{base}/ScannerCapabilities"),
        "",
        &[],
        TIMEOUT,
    )?;
    if response.status != 200 {
        return Err(PrinterError::Refused(format!(
            "no scanner answered at {scanner_uri} (HTTP {})",
            response.status
        )));
    }
    Ok(parse_capabilities(&String::from_utf8_lossy(&response.body)))
}

pub fn parse_capabilities(xml: &str) -> ScannerCapabilities {
    let first = |name: &str| xml_values(xml, name).into_iter().next().unwrap_or_default();
    let mut resolutions: Vec<u32> = xml_values(xml, "XResolution")
        .iter()
        .filter_map(|text| text.parse().ok())
        .collect();
    resolutions.sort_unstable();
    resolutions.dedup();

    ScannerCapabilities {
        make_and_model: first("MakeAndModel"),
        resolutions,
        has_platen: xml.contains("<scan:Platen") || xml.contains("<Platen"),
        has_feeder: xml.contains("<scan:Adf") || xml.contains("<Adf"),
    }
}

#[cfg(test)]
mod tests;
