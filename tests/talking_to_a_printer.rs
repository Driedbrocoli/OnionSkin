//! `onionskin send` and `onionskin printers`, driven against a printer that
//! is not there.
//!
//! The IPP itself is well covered by the library's own tests: requests are
//! built right, replies are parsed right, a refusal comes back in words. What
//! none of that touches is the *wiring* — whether the command somebody types
//! reaches those functions with the right arguments and says something useful
//! about what came back.
//!
//! That gap is worth closing on its own account. The one serious bug found by
//! hand this week was of exactly that shape: `--after` had a working
//! implementation in the library and a command that never called it, so the
//! words were dropped in silence and the library's tests all passed.
//!
//! A printer speaks IPP over HTTP, so a socket on localhost that answers the
//! way a printer answers is enough to drive the whole path — the file is read,
//! a job is built, it goes out over TCP, and the reply is turned into a
//! sentence. Everything but the paper.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_onionskin"))
}

struct Run {
    ok: bool,
    stdout: String,
    stderr: String,
}

impl Run {
    fn said(&self) -> String {
        format!("{}{}", self.stdout, self.stderr)
    }
}

fn run(home: &Path, args: &[&str]) -> Run {
    let output = Command::new(binary())
        .args(args)
        .env("ONIONSKIN_HOME", home)
        .output()
        .expect("the binary should run");
    Run {
        ok: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// A printer that answers once, and hands back what it was asked.
///
/// Reads the head, then exactly as much body as the head says — which is what
/// makes it possible to check that the whole job arrived rather than a prefix
/// of it.
fn a_printer_that_answers(reply: Vec<u8>) -> (String, mpsc::Receiver<Vec<u8>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (send, receive) = mpsc::channel();

    std::thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
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

/// An IPP reply wrapped in the HTTP a printer puts round it.
fn ipp_http(status: u16, attributes: &[(u8, &str, &[u8])]) -> Vec<u8> {
    let mut ipp = vec![0x01, 0x01];
    ipp.extend_from_slice(&status.to_be_bytes());
    ipp.extend_from_slice(&1u32.to_be_bytes());
    ipp.push(0x01); // operation attributes
    for (tag, name, value) in attributes {
        ipp.push(*tag);
        ipp.extend_from_slice(&(name.len() as u16).to_be_bytes());
        ipp.extend_from_slice(name.as_bytes());
        ipp.extend_from_slice(&(value.len() as u16).to_be_bytes());
        ipp.extend_from_slice(value);
    }
    ipp.push(0x03); // end of attributes

    let mut out = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/ipp\r\nContent-Length: {}\r\n\r\n",
        ipp.len()
    )
    .into_bytes();
    out.extend_from_slice(&ipp);
    out
}

/// A one-page delta, made the way a person makes one.
fn a_delta(home: &Path, dir: &Path) -> PathBuf {
    let document = dir.join("sheet.osk").to_string_lossy().into_owned();
    assert!(run(home, &["new", &document, "--page", "a4"]).ok);
    assert!(run(home, &["write", &document, "--at", "20,40:APPROVED"]).ok);
    let pdf = dir.join("delta.pdf");
    let printed = run(home, &["print", &document, "-o", &pdf.to_string_lossy()]);
    assert!(printed.ok, "{}", printed.said());
    pdf
}

/// The whole path, end to end: a file on disk becomes an IPP job on a socket,
/// and what the printer says comes back as a sentence.
#[test]
fn a_delta_reaches_the_printer_and_the_job_number_comes_back() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let delta = a_delta(&home, dir.path());

    let (address, asked) = a_printer_that_answers(ipp_http(
        0x0000,
        &[
            (0x21, "job-id", &7i32.to_be_bytes()),
            (0x21, "job-state", &3i32.to_be_bytes()),
        ],
    ));

    let sent = run(
        &home,
        &[
            "send",
            &delta.to_string_lossy(),
            "--printer",
            &format!("ipp://{address}/ipp/print"),
        ],
    );
    assert!(sent.ok, "the send failed: {}", sent.said());

    // The printer was actually spoken to, and the whole delta arrived.
    let body = asked
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("the printer was never asked for anything");
    assert!(
        body.windows(4).any(|w| w == b"%PDF"),
        "the job that arrived has no PDF in it ({} bytes)",
        body.len()
    );
    let on_disk = std::fs::metadata(&delta).unwrap().len() as usize;
    assert!(
        body.len() >= on_disk,
        "only {} bytes arrived of a {on_disk}-byte delta — the job was truncated",
        body.len()
    );

    // And what came back was turned into something a person can read.
    assert!(
        sent.said().contains('7'),
        "the job number the printer gave was not reported: {}",
        sent.said()
    );
}

/// Printing at anything but 100% is the mistake this whole program exists to
/// avoid, so the job has to say so to the printer rather than trusting a
/// default. Checked on the bytes that actually went out.
#[test]
fn the_job_tells_the_printer_not_to_scale_the_page() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let delta = a_delta(&home, dir.path());

    let (address, asked) =
        a_printer_that_answers(ipp_http(0x0000, &[(0x21, "job-id", &1i32.to_be_bytes())]));

    let sent = run(
        &home,
        &[
            "send",
            &delta.to_string_lossy(),
            "--printer",
            &format!("ipp://{address}/ipp/print"),
        ],
    );
    assert!(sent.ok, "{}", sent.said());

    let body = asked
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("the printer was never asked for anything");
    let found = |needle: &[u8]| body.windows(needle.len()).any(|w| w == needle);
    assert!(
        found(b"print-scaling"),
        "the job never mentions scaling, so the printer will use its own default"
    );
    assert!(found(b"none"), "the job does not ask for no scaling at all");
}

/// A printer that refuses has to be reported in words, and the command must
/// fail — a delta that never printed reported as sent is how somebody files
/// the sheet and finds out later.
#[test]
fn a_printer_that_refuses_is_reported_and_the_command_fails() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let delta = a_delta(&home, dir.path());

    // 0x0501: server-error-internal-error.
    let (address, _asked) = a_printer_that_answers(ipp_http(0x0501, &[]));

    let sent = run(
        &home,
        &[
            "send",
            &delta.to_string_lossy(),
            "--printer",
            &format!("ipp://{address}/ipp/print"),
        ],
    );
    assert!(
        !sent.ok,
        "a refused job was reported as sent: {}",
        sent.said()
    );
    // A sentence first, with the raw code after it in brackets — the sentence
    // is for the person holding the paper, the code is for whoever they ring
    // about it. A number on its own would be neither.
    assert!(
        sent.said().contains("refused the job"),
        "the refusal does not say the job was refused: {}",
        sent.said()
    );
    assert!(
        sent.said()
            .contains("something went wrong inside the printer"),
        "the status came back without being put into words: {}",
        sent.said()
    );
}

/// Nothing listening is the commonest thing that goes wrong — the printer is
/// asleep, or the address has a typo in it. It must say so rather than hang.
#[test]
fn a_printer_that_is_not_there_says_so_rather_than_hanging() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let delta = a_delta(&home, dir.path());

    // A port nothing is listening on: bound, then dropped.
    let address = {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap()
    };

    let sent = run(
        &home,
        &[
            "send",
            &delta.to_string_lossy(),
            "--printer",
            &format!("ipp://{}:{}/ipp/print", address.ip(), address.port()),
        ],
    );
    assert!(!sent.ok, "{}", sent.said());
    assert!(
        sent.said().to_lowercase().contains("could not")
            || sent.said().to_lowercase().contains("no answer")
            || sent.said().to_lowercase().contains("refused"),
        "the failure does not say what went wrong: {}",
        sent.said()
    );
}

/// A file that is not there must be caught before a printer is troubled with
/// it — and certainly before a job is half sent.
#[test]
fn a_file_that_is_not_there_is_caught_before_anything_is_sent() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");

    let sent = run(
        &home,
        &[
            "send",
            &dir.path().join("nowhere.pdf").to_string_lossy(),
            "--printer",
            "ipp://127.0.0.1:9/ipp/print",
        ],
    );
    assert!(!sent.ok, "{}", sent.said());
    assert!(
        sent.said().contains("nowhere.pdf"),
        "the refusal does not name the file: {}",
        sent.said()
    );
}

/// `doctor` is the report somebody reads to find out what is set up here, so a
/// printer that has been set has to be on it.
///
/// Left off, the report shows a made-up example address beside a real one
/// sitting in the settings, and is less use than `config show` — while looking
/// like the place to find out.
#[test]
fn doctor_names_the_printer_that_was_set_and_says_how_to_set_one_when_none_is() {
    let home = tempfile::tempdir().unwrap();

    // Nothing set: the example, and how to stop needing it.
    let bare = run(home.path(), &["doctor"]);
    let said = bare.said();
    assert!(said.contains("to any network printer"), "{said}");
    assert!(said.contains("config set printer"), "{said}");

    let set = run(
        home.path(),
        &["config", "set", "printer", "ipp://office/laser"],
    );
    assert!(set.ok, "{}", set.said());
    let set = run(
        home.path(),
        &["config", "set", "scanner", "http://office/eSCL"],
    );
    assert!(set.ok, "{}", set.said());

    let after = run(home.path(), &["doctor"]);
    let said = after.said();
    assert!(
        said.contains("ipp://office/laser"),
        "the printer that was set is not on the report: {said}"
    );
    assert!(
        said.contains("http://office/eSCL"),
        "the scanner that was set is not on the report: {said}"
    );
    // And the example that is no longer needed is gone, rather than sitting
    // beside the real one saying two different things.
    assert!(
        !said.contains("ipp://printer.local/ipp/print"),
        "the made-up address is still there beside the real one: {said}"
    );
}

/// A printer that answers with something other than an answer.
///
/// This is the one part of Onionskin that talks to a machine it did not write
/// and cannot see. A printer on an office network may be twelve years old, may
/// be a print server pretending to be a printer, or may be something else
/// entirely on the port somebody typed — and whatever comes back down the socket
/// is parsed before anything has had a chance to check it.
///
/// What is being asked of it is not that it understands these. It is that it
/// **comes back**. A hang is worse than a crash here: a crash says something is
/// wrong, and a program stopped at a socket with a job half sent says nothing at
/// all while somebody waits for their printing.
///
/// It does come back. The one worth naming is a printer that promises a hundred
/// thousand bytes and sends five, then holds the socket open — Onionskin waits
/// out its own sixty-second read timeout and reports the printer unreachable,
/// which is the right answer and takes a full minute to give. That minute is
/// deliberate and documented in `printer`: a printer waking from sleep takes its
/// time, and a spurious timeout looks exactly like a printer that is switched
/// off. So this test allows longer than the program's own patience rather than
/// calling patience a hang.
fn a_printer_that_answers_badly(reply: Vec<u8>, then_hang: bool) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        // Read what it feels like reading and no more, then answer.
        let mut scratch = [0u8; 4096];
        let _ = stream.read(&mut scratch);
        let _ = stream.write_all(&reply);
        let _ = stream.flush();
        if then_hang {
            // Promised a body and never sends it. The socket stays open, which
            // is the case a client waiting for `Content-Length` bytes never
            // wakes up from.
            std::thread::sleep(std::time::Duration::from_secs(90));
        }
    });
    format!("{}:{}", address.ip(), address.port())
}

/// Run the binary, and insist it finishes.
///
/// `output()` waits for ever, which is exactly the failure being looked for —
/// so the child is watched and killed, and a kill is a failed test rather than a
/// quiet pass.
fn run_before(home: &Path, args: &[&str], seconds: u64) -> Result<String, String> {
    let mut child = Command::new(binary())
        .args(args)
        .env("ONIONSKIN_HOME", home)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("the binary should start");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(seconds);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(50))
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("still going after {seconds} seconds"));
            }
            Err(why) => return Err(why.to_string()),
        }
    }
    let out = child.wait_with_output().expect("the binary should finish");
    Ok(format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    ))
}

#[test]
fn nothing_a_printer_can_answer_leaves_onionskin_waiting() {
    let dir = tempfile::tempdir().expect("a place to work");
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).expect("a home of its own");
    let delta = a_delta(&home, dir.path());

    let replies: Vec<(&str, Vec<u8>, bool)> = vec![
        ("nothing at all", Vec::new(), false),
        ("a closed socket", Vec::new(), false),
        (
            "headers and no body",
            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n".to_vec(),
            false,
        ),
        (
            "a promise it does not keep",
            b"HTTP/1.1 200 OK\r\nContent-Length: 100000\r\n\r\nshort".to_vec(),
            true,
        ),
        (
            "a web page",
            b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 44\r\n\r\n\
              <html><body>Printer web interface</body></html>"
                .to_vec(),
            false,
        ),
        (
            "an IPP reply cut off mid-attribute",
            [
                b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\n\r\n".to_vec(),
                vec![0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x01],
            ]
            .concat(),
            false,
        ),
        (
            "an IPP attribute longer than the reply",
            [
                b"HTTP/1.1 200 OK\r\nContent-Length: 14\r\n\r\n".to_vec(),
                vec![
                    0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x01, 0x47, 0xFF, 0xFF, 0x41,
                    0x42,
                ],
            ]
            .concat(),
            false,
        ),
        (
            "every byte there is",
            [
                b"HTTP/1.1 200 OK\r\nContent-Length: 256\r\n\r\n".to_vec(),
                (0u8..=255).collect(),
            ]
            .concat(),
            false,
        ),
        ("not http at all", b"greetings\n".to_vec(), false),
        (
            "a status nobody defines",
            b"HTTP/1.1 799 Nonsense\r\nContent-Length: 0\r\n\r\n".to_vec(),
            false,
        ),
        (
            // This one used to be a panic rather than a message: the chunk
            // walker stepped over a CRLF nobody had checked was there.
            "a chunked reply cut off after a chunk's data",
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nABCD".to_vec(),
            false,
        ),
        (
            "a chunked reply cut off inside a chunk",
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n99\r\nAB".to_vec(),
            false,
        ),
    ];

    for (name, reply, then_hang) in replies {
        let address = a_printer_that_answers_badly(reply, then_hang);
        let said = run_before(
            &home,
            &[
                "send",
                delta.to_str().unwrap(),
                "--printer",
                &format!("ipp://{address}/ipp/print"),
            ],
            // Longer than the program's own sixty-second read timeout, so a
            // failure here means it really never came back.
            90,
        )
        .unwrap_or_else(|why| panic!("'{name}': {why}"));

        // Whatever it decided, it has to have said something a person can act
        // on rather than stopping with an empty screen.
        assert!(
            said.trim().len() > 10,
            "'{name}' came back with almost nothing to say: {said:?}"
        );
    }
}

/// The sentence that matters most in the whole program.
///
/// Onionskin exists to put a few words onto a sheet that has already been
/// through the printer once. The document is written to the socket *before*
/// the reply is read, so anything that goes wrong while reading the reply
/// happens with the delta already in the printer's hands and, quite possibly,
/// with the sheet moving. "It didn't work, try again" is then the most
/// expensive advice there is: the sheet comes back with the addition on it
/// twice, and toner does not lift.
///
/// So a failure on that side of the send says the job may already have been
/// taken, and a failure on the other side — a printer that was never reached,
/// or one that answered and refused — does not, because a warning everybody
/// sees is a warning nobody reads.
#[test]
fn a_failure_after_the_delta_was_sent_says_the_sheet_may_be_printing() {
    let dir = tempfile::tempdir().expect("a place to work");
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).expect("a home of its own");
    let delta = a_delta(&home, dir.path());

    let send_to = |address: String| -> String {
        run_before(
            &home,
            &[
                "send",
                delta.to_str().unwrap(),
                "--printer",
                &format!("ipp://{address}/ipp/print"),
            ],
            90,
        )
        .expect("it should come back")
    };

    // The printer took the document and then stopped talking.
    for reply in [
        &b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nABCD"[..],
        &b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\nnope"[..],
        &b"greetings\n"[..],
    ] {
        let said = send_to(a_printer_that_answers_badly(reply.to_vec(), false));
        assert!(
            said.contains("may be printing"),
            "no warning about the sheet: {said}"
        );
        assert!(
            said.contains("cannot be undone"),
            "no warning about the sheet: {said}"
        );
    }

    // The printer answered and refused. Nothing is printing, and saying it
    // might be would train people to disregard the warning above.
    let refused = ipp_http(200, &[(0x21, "status-code", &0x0400u32.to_be_bytes()[..])]);
    let (address, _sent) = a_printer_that_answers(refused);
    let said = send_to(address.trim_start_matches("ipp://").to_string());
    assert!(
        !said.contains("may be printing"),
        "warned about a sheet that is not printing: {said}"
    );
}
