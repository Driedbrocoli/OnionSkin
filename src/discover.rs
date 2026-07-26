//! Finding the printers and scanners on this network, so nobody has to type an
//! address.
//!
//! Asking somebody for their printer's address is asking them for something
//! they have never had a reason to know. It is on a status page three menus
//! into the machine's own screen, it changes when the router is restarted, and
//! the first thing anybody tries is the name printed on the front of the
//! printer, which is not it. Every question answered here is a question the
//! person at the keyboard should never have been asked.
//!
//! # How a printer says where it is
//!
//! Printers and scanners announce themselves with DNS-SD over multicast DNS —
//! Bonjour on a Mac, Avahi on Linux, and just "network printing" in the manual.
//! It is ordinary DNS records sent to an address every machine on the same
//! cable or the same Wi-Fi hears. A printer publishes a pointer record saying
//! that a printer called Office exists, a service record saying which host and
//! port it answers on, a text record with the details — including the path to
//! send jobs to — and an address record for the host. Ask for the pointer and a
//! well-behaved machine sends the whole set back at once.
//!
//! All of that is spoken here over a plain `std::net::UdpSocket`. A DNS message
//! is a twelve-byte header and a list of records, and the only part of it that
//! is not obvious is name compression, where a name that has already appeared
//! in the packet is written as a two-byte offset pointing back at it. That is
//! worth the hundred lines below. It is not worth a dependency: the libraries
//! for this bring a general resolver, an asynchronous runtime and a service
//! registry with them, which is a great deal of network for a program whose
//! whole promise is that it stays off it.
//!
//! # About the network
//!
//! Onionskin never phones home, and this is the one place where it speaks
//! first, so it is worth being exact. The only address anything here sends to
//! is 224.0.0.251, the group reserved for multicast DNS. That group is
//! link-local by definition — no router forwards it — so the question reaches
//! the machines on your own cable or your own Wi-Fi and stops there. Nothing
//! goes out unless somebody asks Onionskin to look, nothing is written down
//! afterwards, and there is no name, document or identifier of yours anywhere
//! in the packet. It is six questions of the form "is there a printer?".
//!
//! # Why nothing here returns an error
//!
//! There are a dozen ordinary reasons for finding nothing: multicast blocked by
//! a firewall, a Wi-Fi access point that filters it, a virtual machine on a NAT
//! network, a laptop with the network off, a locked-down account that may not
//! open a socket at all, or a printer plugged in by USB that was never on the
//! network in the first place. All of them look the same to the person waiting,
//! which is that no printer appeared. An empty list says exactly that and
//! leaves them where they were — typing the address, which still works. An
//! error would turn a quiet network into a program that will not go on.

use std::collections::BTreeMap;
use std::net::{Ipv4Addr, UdpSocket};
use std::time::{Duration, Instant};

/// How long to listen before giving up on the rest.
///
/// Two seconds is long enough for everything on a home or office link to have
/// answered, and short enough that somebody watching a list fill in does not
/// wonder whether the button worked. Printers answer in tens of milliseconds;
/// what the rest of the time is for is the machine that was asleep.
pub const LISTEN_FOR: Duration = Duration::from_secs(2);

/// The floor and ceiling on how long a caller may ask us to listen for.
///
/// Clamped rather than refused, because this returns a list and never an error.
/// The ceiling matters most: a window that called this on its own thread with a
/// wrong number in it would sit there, apparently broken, for as long as the
/// number said.
const SHORTEST: Duration = Duration::from_millis(100);
const LONGEST: Duration = Duration::from_secs(30);

/// The group every responder listens on, and the only address this sends to.
const GROUP: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 251);
const PORT: u16 = 5353;

/// The fixed part at the front of every DNS message: an identifier, the flags,
/// and the four counts of what follows.
const HEADER: usize = 12;

/// The largest reply worth reading, in bytes.
///
/// Multicast DNS allows a message the size of the interface's MTU, and 9000 is
/// the largest anything sends. A device claiming more than that is not a device
/// this program needs to talk to.
const MOST_A_REPLY_MAY_HOLD: usize = 9000;

/// How far a name may be chased through compression pointers before the packet
/// is decided to be lying.
///
/// A pointer that points at itself, or at another pointer that points back, is
/// a loop, and following it is a program that hangs on a single malformed
/// packet — one that anybody on the network can send. Real names are compressed
/// once or twice; thirty-two is far past anything honest.
const MOST_JUMPS: usize = 32;

/// A DNS name is 255 bytes at the outside. This is the second half of the loop
/// guard: a packet can point around a long way without ever repeating itself,
/// and the length is what stops that.
const MOST_A_NAME_MAY_HOLD: usize = 255;

/// How many records to take in before deciding the link is not being honest.
///
/// The listening time already bounds this. The count bounds the memory, so that
/// somebody flooding the network with answers costs a fixed amount rather than
/// as much as they care to send in two seconds.
const MOST_RECORDS: usize = 4096;

/// How many devices to ask a second question of. See [`chase`].
const MOST_TO_CHASE: usize = 16;

/// The record types worth reading, from RFC 1035 and RFC 2782.
mod record_type {
    pub const A: u16 = 1;
    pub const PTR: u16 = 12;
    pub const TXT: u16 = 16;
    pub const SRV: u16 = 33;
}

// ---------------------------------------------------------------------------
// What we are looking for
// ---------------------------------------------------------------------------

/// Which half of the job a device can do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum Kind {
    Printer,
    Scanner,
}

impl Kind {
    pub fn name(&self) -> &'static str {
        match self {
            Kind::Printer => "printer",
            Kind::Scanner => "scanner",
        }
    }
}

/// One service type, and what an address for it looks like.
struct Service {
    /// The name it is advertised under, without the trailing dot.
    advertised: &'static str,
    kind: Kind,
    /// Whether talking to it means TLS.
    encrypted: bool,
    scheme: &'static str,
    /// The text record key holding the path, where the service has one.
    path_key: &'static str,
    /// The path to assume when the device does not say. Every one of these is
    /// what the overwhelming majority of machines use, so a device that answers
    /// with a bare service record still gives an address worth trying.
    path: &'static str,
    /// The port the scheme implies, which is left out of the address when it
    /// matches — `ipp://printer/ipp/print` is what people expect to see.
    usual_port: u16,
}

/// Everything worth asking about.
///
/// Printers first, then scanners, because that is the order they are offered
/// in and a list that changes order between runs is a list nobody trusts.
const SERVICES: [Service; 6] = [
    Service {
        advertised: "_ipp._tcp.local",
        kind: Kind::Printer,
        encrypted: false,
        scheme: "ipp",
        path_key: "rp",
        path: "ipp/print",
        usual_port: 631,
    },
    Service {
        advertised: "_ipps._tcp.local",
        kind: Kind::Printer,
        encrypted: true,
        scheme: "ipps",
        path_key: "rp",
        path: "ipp/print",
        usual_port: 631,
    },
    // The bare socket every printer has had since the nineties: no protocol at
    // all, just the file down a TCP connection. Kept because a great many
    // printers on an office network advertise this and nothing else, and
    // knowing the machine is there beats not knowing.
    Service {
        advertised: "_pdl-datastream._tcp.local",
        kind: Kind::Printer,
        encrypted: false,
        scheme: "socket",
        path_key: "",
        path: "",
        usual_port: 9100,
    },
    Service {
        advertised: "_uscan._tcp.local",
        kind: Kind::Scanner,
        encrypted: false,
        scheme: "http",
        path_key: "rs",
        path: "eSCL",
        usual_port: 80,
    },
    Service {
        advertised: "_uscans._tcp.local",
        kind: Kind::Scanner,
        encrypted: true,
        scheme: "https",
        path_key: "rs",
        path: "eSCL",
        usual_port: 443,
    },
    // Advertised by machines that scan but do not say how. Onionskin guesses
    // eSCL because eSCL is the only scanning protocol it speaks, and
    // [`crate::printer::scanner_present`] settles the guess in a second — which
    // is a better offer than leaving the machine out of the list entirely.
    Service {
        advertised: "_scanner._tcp.local",
        kind: Kind::Scanner,
        encrypted: false,
        scheme: "http",
        path_key: "rs",
        path: "eSCL",
        usual_port: 80,
    },
];

/// A device that answered.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Found {
    /// What it calls itself, in the words on the front of the machine — "HP
    /// LaserJet 400 (A1B2C3)" rather than a hostname.
    pub name: String,
    /// The host it named, usually something ending in `.local`.
    pub host: String,
    /// Where that host actually is, when the reply said.
    ///
    /// Optional because a device may answer with a name and leave the address
    /// to a second question that never gets answered. It is worth having
    /// separately from the address string: a `.local` name resolves only on a
    /// machine running a responder of its own, and plenty of Linux installs
    /// have none, so the number is what makes the address in [`Found::uri`]
    /// work everywhere.
    pub address: Option<Ipv4Addr>,
    pub port: u16,
    pub kind: Kind,
    /// Whether the service it was advertised under means TLS.
    pub encrypted: bool,
    /// The text record, key by value, with the keys lowercased — they are not
    /// case sensitive and half the printers on the market disagree about it.
    pub txt: Vec<(String, String)>,
    /// An address that can be handed straight to [`crate::printer`].
    pub uri: String,
}

impl Found {
    /// One value out of the text record, whatever case the device wrote it in.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.txt
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(key))
            .map(|(_, value)| value.as_str())
    }

    /// The make and model, if the device offered one.
    ///
    /// It travels under the key `ty`, which nobody would guess, which is why
    /// this is here rather than left to the caller.
    pub fn model(&self) -> Option<&str> {
        self.get("ty").filter(|text| !text.is_empty())
    }

    /// Where somebody said the machine is — "second floor", "by the kitchen".
    /// Under the key `note`, which is no more guessable than `ty`.
    pub fn location(&self) -> Option<&str> {
        self.get("note").filter(|text| !text.is_empty())
    }

    /// The same address without the encryption, for a caller that cannot speak
    /// it.
    ///
    /// Onionskin's printing does not do TLS yet, and a printer advertising
    /// `_ipps` almost always accepts plain IPP on the same port as well — the
    /// encrypted service is an offer rather than a replacement. So a caller
    /// holding an address it cannot use has somewhere to go rather than
    /// nowhere. An address that is already unencrypted comes back unchanged,
    /// and so does an `https` scanner: stepping that down would point at a port
    /// that speaks TLS and nothing else, which is a worse answer than the true
    /// one.
    pub fn plain_uri(&self) -> String {
        match self.uri.split_once("://") {
            Some(("ipps", rest)) => format!("ipp://{rest}"),
            _ => self.uri.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// Asking
// ---------------------------------------------------------------------------

/// Every printer and scanner that answers within `listen_for`.
pub fn find(listen_for: Duration) -> Vec<Found> {
    look(None, listen_for)
}

/// Only the printers, which is two fewer questions on the wire and a list with
/// nothing in it the caller would have to filter out again.
pub fn printers(listen_for: Duration) -> Vec<Found> {
    look(Some(Kind::Printer), listen_for)
}

/// Only the scanners.
pub fn scanners(listen_for: Duration) -> Vec<Found> {
    look(Some(Kind::Scanner), listen_for)
}

/// The sockets the questions go out on.
struct Sockets {
    /// The one that asks. It is bound to whatever port the machine hands out
    /// rather than to 5353, and that is deliberate: a responder receiving a
    /// query from any other port treats it as a one-shot question and sends the
    /// answer straight back to that port. Which means this needs no share of
    /// 5353, and works on a machine already running Avahi or Bonjour — that is
    /// to say, on nearly every machine.
    asking: UdpSocket,
    /// A second one on 5353 itself, when nothing else on the machine has it.
    ///
    /// Some devices answer a one-shot question by multicast anyway, and the
    /// answer then goes to 5353 where the socket above will never see it. This
    /// catches those. It usually cannot be opened at all — Avahi and Bonjour
    /// hold that port for the whole machine — and when it cannot, the answers
    /// coming back to the socket above are enough.
    listening: Option<UdpSocket>,
}

impl Sockets {
    fn each(&self) -> impl Iterator<Item = &UdpSocket> {
        std::iter::once(&self.asking).chain(self.listening.iter())
    }
}

fn open() -> Option<Sockets> {
    let asking = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    // RFC 6762 asks for 255 so that a responder can tell a message came from
    // the local link and was not routed to it.
    let _ = asking.set_multicast_ttl_v4(255);
    // And this is what lets a responder on *this* machine hear the question —
    // a printer shared by CUPS from here is announced by the machine's own
    // Avahi, and without the loopback we would find every printer but that one.
    let _ = asking.set_multicast_loop_v4(true);

    let listening = match UdpSocket::bind((Ipv4Addr::UNSPECIFIED, PORT)) {
        Ok(socket)
            if socket
                .join_multicast_v4(&GROUP, &Ipv4Addr::UNSPECIFIED)
                .is_ok() =>
        {
            Some(socket)
        }
        _ => None,
    };
    Some(Sockets { asking, listening })
}

fn look(only: Option<Kind>, listen_for: Duration) -> Vec<Found> {
    let listen_for = listen_for.clamp(SHORTEST, LONGEST);
    let Some(sockets) = open() else {
        return Vec::new();
    };
    let deadline = Instant::now() + listen_for;

    for service in SERVICES.iter().filter(|service| wanted(service, only)) {
        ask(&sockets, &question(service.advertised, record_type::PTR));
    }

    let mut records = Vec::new();
    gather(&sockets, Instant::now() + listen_for / 2, &mut records);
    for question in chase(&records, only) {
        ask(&sockets, &question);
    }
    gather(&sockets, deadline, &mut records);

    merge(assemble(&records, only))
}

fn wanted(service: &Service, only: Option<Kind>) -> bool {
    only.is_none() || only == Some(service.kind)
}

/// Send one question, and shrug if it will not go.
///
/// Every failure here is a machine with no network, no route to the multicast
/// group, or a firewall in the way. All three mean the same thing — that
/// nothing is going to answer — and none of them is worth stopping for.
fn ask(sockets: &Sockets, message: &[u8]) {
    let _ = sockets.asking.send_to(message, (GROUP, PORT));
}

/// One question, as the bytes that go on the wire.
fn question(name: &str, kind: u16) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(name.len() + HEADER + 6);
    bytes.extend_from_slice(&query_id().to_be_bytes());
    // No flags at all. Recursion is meaningless on a link where every machine
    // answers for itself, and asking for it confuses the ones that check.
    bytes.extend_from_slice(&0u16.to_be_bytes());
    bytes.extend_from_slice(&1u16.to_be_bytes()); // one question
    bytes.extend_from_slice(&0u16.to_be_bytes()); // and no answers of our own
    bytes.extend_from_slice(&0u16.to_be_bytes());
    bytes.extend_from_slice(&0u16.to_be_bytes());
    encode_name(&mut bytes, name);
    bytes.extend_from_slice(&kind.to_be_bytes());
    // Class IN, with the top bit set. That bit asks for the answer to come
    // straight back to this socket instead of to the whole link, which spares
    // every other machine on it a packet it has no use for.
    bytes.extend_from_slice(&0x8001u16.to_be_bytes());
    bytes
}

/// A number to tell one conversation from another.
///
/// Nothing here checks it coming back, because an answer sent to the whole link
/// carries a zero rather than an echo. It is here so that two copies of
/// Onionskin asking at the same moment do not look to a responder like one copy
/// asking twice.
fn query_id() -> u16 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.subsec_nanos() as u16)
        .unwrap_or(0)
}

/// Listen until `until`, reading everything that arrives into `records`.
fn gather(sockets: &Sockets, until: Instant, records: &mut Vec<Record>) {
    let mut buffer = vec![0u8; MOST_A_REPLY_MAY_HOLD];
    while Instant::now() < until {
        for socket in sockets.each() {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return;
            }
            // A slice at a time rather than the whole remaining wait, so that a
            // second socket with something to say is not left unread until the
            // first one times out.
            let _ = socket.set_read_timeout(Some(left.min(Duration::from_millis(100))));
            while let Ok((count, _from)) = socket.recv_from(&mut buffer) {
                records.extend(parse(&buffer[..count]));
                if records.len() >= MOST_RECORDS || Instant::now() >= until {
                    return;
                }
            }
        }
    }
}

/// The second round of questions: what to ask about again, and directly.
///
/// A responder is supposed to send the service, text and address records along
/// with the pointer, and most do. The ones that do not leave a name with
/// nothing behind it — a printer that appears in a list and cannot be printed
/// to — so those get asked about by name. It is capped because a link with a
/// hundred devices on it should not turn one round of questions into two
/// hundred.
fn chase(records: &[Record], only: Option<Kind>) -> Vec<Vec<u8>> {
    let mut questions = Vec::new();
    let mut asked: Vec<String> = Vec::new();

    // A name that was pointed at and never explained.
    for record in records {
        let Record::Pointer { instance, .. } = record else {
            continue;
        };
        let lower = instance.to_ascii_lowercase();
        if service_of(&lower, only).is_none() || asked.contains(&lower) {
            continue;
        }
        let explained = records.iter().any(|other| match other {
            Record::Service {
                instance: named, ..
            } => named.eq_ignore_ascii_case(instance),
            _ => false,
        });
        if explained || questions.len() + 2 > MOST_TO_CHASE {
            continue;
        }
        asked.push(lower);
        questions.push(question(instance, record_type::SRV));
        questions.push(question(instance, record_type::TXT));
    }

    // And a host that was named and never placed.
    for record in records {
        let Record::Service { host, .. } = record else {
            continue;
        };
        let lower = host.to_ascii_lowercase();
        if asked.contains(&lower) || questions.len() >= MOST_TO_CHASE {
            continue;
        }
        let placed = records.iter().any(|other| match other {
            Record::Address { host: named, .. } => named.eq_ignore_ascii_case(host),
            _ => false,
        });
        if !placed {
            asked.push(lower);
            questions.push(question(host, record_type::A));
        }
    }
    questions
}

// ---------------------------------------------------------------------------
// Reading a DNS message
// ---------------------------------------------------------------------------

/// One record, in the only four shapes this program has any use for.
#[derive(Debug, Clone, PartialEq)]
enum Record {
    /// "There is a printer called this."
    Pointer { service: String, instance: String },
    /// "It is on this host, at this port."
    Service {
        instance: String,
        host: String,
        port: u16,
    },
    /// "And here is what else it wants you to know."
    Text {
        instance: String,
        pairs: Vec<(String, String)>,
    },
    /// "That host is at this address."
    Address { host: String, address: Ipv4Addr },
}

/// Everything a message says, as far as it can be read.
///
/// Never an error and never a panic. What arrives on this socket is whatever
/// anybody on the network chose to send: truncated by a dropped fragment,
/// written by a printer's firmware with its own ideas about the format, or
/// built deliberately to see what happens. So every field is checked against
/// the length of what actually arrived, and a message that stops making sense
/// halfway through is read up to that point and no further. The records read
/// before the damage are still true, and throwing them away would lose a
/// printer over a mangled record belonging to something else.
///
/// The question section is skipped rather than read. A query carries no
/// answers, so one arriving here contributes nothing and needs no special
/// handling — except that a query may carry records in its answer section, to
/// tell other responders what it already knows. Those are perfectly good
/// statements about devices on the link, and are read like any other.
fn parse(message: &[u8]) -> Vec<Record> {
    let mut records = Vec::new();
    if message.len() < HEADER {
        return records;
    }
    let count = |at: usize| u16::from_be_bytes([message[at], message[at + 1]]) as usize;
    let questions = count(4);
    // Answers, authority and additional together: a printer scatters its
    // records across all three sections and which one a record arrived in makes
    // no difference to what it says.
    let answers = count(6) + count(8) + count(10);

    let mut at = HEADER;
    for _ in 0..questions {
        let Some((_, next)) = read_name(message, at) else {
            return records;
        };
        // The type and class after the name, neither of which is needed.
        at = next + 4;
    }

    for _ in 0..answers {
        let Some((name, next)) = read_name(message, at) else {
            break;
        };
        let Some(head) = message.get(next..next + 10) else {
            break;
        };
        let kind = u16::from_be_bytes([head[0], head[1]]);
        let class = u16::from_be_bytes([head[2], head[3]]);
        let ttl = u32::from_be_bytes([head[4], head[5], head[6], head[7]]);
        let length = u16::from_be_bytes([head[8], head[9]]) as usize;
        let from = next + 10;
        let Some(data) = message.get(from..from + length) else {
            break;
        };
        at = from + length;

        // The top bit of the class is the cache-flush flag, which says nothing
        // about what the record means. Everything else must be class IN.
        if class & 0x7fff != 1 {
            continue;
        }
        // A record with no time to live is a goodbye: the device is announcing
        // that it is going away. Listing it would offer somebody a printer that
        // has just been switched off.
        if ttl == 0 {
            continue;
        }

        match kind {
            record_type::PTR => {
                if let Some((instance, _)) = read_name(message, from) {
                    records.push(Record::Pointer {
                        service: name,
                        instance,
                    });
                }
            }
            record_type::SRV => {
                // Priority and weight, then the port, then the host — and the
                // host is a name like any other, so it may be a pointer back
                // into the packet rather than spelled out.
                if data.len() >= 6 {
                    let port = u16::from_be_bytes([data[4], data[5]]);
                    if let Some((host, _)) = read_name(message, from + 6) {
                        records.push(Record::Service {
                            instance: name,
                            host,
                            port,
                        });
                    }
                }
            }
            record_type::TXT => records.push(Record::Text {
                instance: name,
                pairs: text_pairs(data),
            }),
            record_type::A => {
                if let Ok(octets) = <[u8; 4]>::try_from(data) {
                    records.push(Record::Address {
                        host: name,
                        address: Ipv4Addr::from(octets),
                    });
                }
            }
            _ => {}
        }
    }
    records
}

/// Read the name at `at`, and say where the record carries on afterwards.
///
/// Names are compressed: a name that has already appeared in the packet is
/// written as two bytes holding an offset back to it, marked by the top two
/// bits being set. Responses lean on this heavily — a reply about one printer
/// mentions the same service name four times — so a parser that does not
/// follow the pointers reads almost nothing.
///
/// Two things stop a hostile or broken packet dead. A pointer may point
/// anywhere, including at itself, so the jumps are counted; and a packet can
/// point around a long way without ever repeating itself, so the length is
/// capped as well. Where the record carries on is remembered from the *first*
/// jump, because that is where the name's own bytes ended.
fn read_name(message: &[u8], from: usize) -> Option<(String, usize)> {
    let mut name = String::new();
    let mut at = from;
    let mut after: Option<usize> = None;
    let mut jumps = 0usize;

    loop {
        let length = *message.get(at)? as usize;
        match length & 0xc0 {
            0 => {
                at += 1;
                if length == 0 {
                    return Some((name, after.unwrap_or(at)));
                }
                let label = message.get(at..at + length)?;
                if !name.is_empty() {
                    name.push('.');
                }
                push_label(&mut name, label);
                if name.len() > MOST_A_NAME_MAY_HOLD {
                    return None;
                }
                at += length;
            }
            0xc0 => {
                let low = *message.get(at + 1)? as usize;
                jumps += 1;
                if jumps > MOST_JUMPS {
                    return None;
                }
                if after.is_none() {
                    after = Some(at + 2);
                }
                at = ((length & 0x3f) << 8) | low;
            }
            // The other two forms were reserved forty years ago and never used.
            _ => return None,
        }
    }
}

/// Add one label to a name being joined up.
///
/// A label may contain a dot — "Office. Second floor" is a perfectly legal
/// printer name — so joining labels with dots and saying nothing loses the
/// difference between a dot in a name and the dot between two names. The
/// escaping is the usual DNS presentation form, and [`split_labels`] takes it
/// back off. Anything that is not a dot or a backslash goes through as it is,
/// so a name in Greek or Japanese stays readable rather than becoming a row of
/// numeric escapes.
fn push_label(name: &mut String, label: &[u8]) {
    for character in String::from_utf8_lossy(label).chars() {
        if character == '.' || character == '\\' {
            name.push('\\');
        }
        name.push(character);
    }
}

/// Split a name back into its labels, undoing that escaping.
///
/// The numeric form (`\032` for a space) is understood as well, because other
/// tools write names that way and a name may reach this from somewhere other
/// than the wire.
fn split_labels(name: &str) -> Vec<String> {
    let mut labels = Vec::new();
    let mut label = String::new();
    let mut characters = name.chars().peekable();

    while let Some(character) = characters.next() {
        match character {
            '.' => labels.push(std::mem::take(&mut label)),
            '\\' => match characters.next() {
                Some(first) if first.is_ascii_digit() => {
                    let mut value = first.to_digit(10).unwrap_or(0);
                    for _ in 0..2 {
                        match characters.peek().and_then(|next| next.to_digit(10)) {
                            Some(digit) => {
                                value = value * 10 + digit;
                                characters.next();
                            }
                            None => break,
                        }
                    }
                    if let Ok(byte) = u8::try_from(value) {
                        label.push(byte as char);
                    }
                }
                Some(other) => label.push(other),
                None => {}
            },
            other => label.push(other),
        }
    }
    labels.push(label);
    // An empty label means the root, or a name written with two dots in a row.
    // Neither is worth carrying around.
    labels.retain(|label| !label.is_empty());
    labels
}

/// Write a name out as the labels a DNS message holds.
fn encode_name(out: &mut Vec<u8>, name: &str) {
    for label in split_labels(name) {
        let bytes = label.as_bytes();
        // The length is one byte, so 63 is as long as a label can be. Cutting
        // rather than refusing keeps this from having a failure case: nothing
        // Onionskin asks about is anywhere near that long.
        let bytes = &bytes[..bytes.len().min(63)];
        out.push(bytes.len() as u8);
        out.extend_from_slice(bytes);
    }
    out.push(0);
}

/// The instance name at the front of `Office._ipp._tcp.local`, as something to
/// show somebody.
fn friendly(instance: &str) -> String {
    split_labels(instance)
        .into_iter()
        .next()
        .unwrap_or_default()
}

/// The key and value pairs inside a text record.
///
/// A text record is not one string but a list of them, each with its length in
/// front, and a printer routinely sends a dozen. A pair is written `key=value`;
/// a string with no equals sign in it is a key on its own, which DNS-SD defines
/// as present-but-empty rather than absent, and the two mean different things
/// to a printer.
fn text_pairs(data: &[u8]) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    let mut at = 0usize;
    while at < data.len() {
        let length = data[at] as usize;
        at += 1;
        let Some(entry) = data.get(at..at + length) else {
            break;
        };
        at += length;
        if entry.is_empty() {
            continue;
        }
        let text = String::from_utf8_lossy(entry);
        // Only the first equals sign separates them: a value may hold one, and
        // `adminurl=http://printer/a=b` is a real thing to receive.
        let (key, value) = match text.split_once('=') {
            Some((key, value)) => (key, value),
            None => (text.as_ref(), ""),
        };
        if key.is_empty() {
            continue;
        }
        pairs.push((key.to_ascii_lowercase(), value.to_string()));
    }
    pairs
}

// ---------------------------------------------------------------------------
// Turning records into devices
// ---------------------------------------------------------------------------

/// What has been heard about one instance so far, from however many records.
#[derive(Debug, Default)]
struct Instance {
    /// The name as the device wrote it, escaping and all.
    name: String,
    /// The service type of the pointer that named it, as a second opinion. The
    /// name itself normally ends in the service type, but a device renamed by a
    /// print server, or one with firmware that is careless about it, does not
    /// always agree — and the pointer record's own name never lies about which
    /// question it is answering.
    announced_as: String,
    host: Option<String>,
    port: u16,
    txt: Vec<(String, String)>,
}

fn assemble(records: &[Record], only: Option<Kind>) -> Vec<Found> {
    let mut instances: BTreeMap<String, Instance> = BTreeMap::new();
    let mut addresses: BTreeMap<String, Ipv4Addr> = BTreeMap::new();

    for record in records {
        match record {
            Record::Pointer { service, instance } => {
                let known = entry(&mut instances, instance);
                if known.announced_as.is_empty() {
                    known.announced_as = service.to_ascii_lowercase();
                }
            }
            Record::Service {
                instance,
                host,
                port,
            } => {
                let known = entry(&mut instances, instance);
                known.host = Some(host.clone());
                known.port = *port;
            }
            Record::Text { instance, pairs } => {
                let known = entry(&mut instances, instance);
                for (key, value) in pairs {
                    if !known.txt.iter().any(|(seen, _)| seen == key) {
                        known.txt.push((key.clone(), value.clone()));
                    }
                }
            }
            Record::Address { host, address } => {
                if usable(*address) {
                    addresses
                        .entry(host.to_ascii_lowercase())
                        .or_insert(*address);
                }
            }
        }
    }

    let mut found = Vec::new();
    for instance in instances.values() {
        let lower = instance.name.to_ascii_lowercase();
        let Some(service) =
            service_of(&lower, only).or_else(|| service_named(&instance.announced_as, only))
        else {
            continue;
        };
        // Without a service record there is no port, and without a port there
        // is nothing to talk to. A name on its own is not a device somebody can
        // be offered.
        let Some(host) = instance.host.clone().filter(|host| !host.is_empty()) else {
            continue;
        };
        if instance.port == 0 {
            continue;
        }

        let address = addresses.get(&host.to_ascii_lowercase()).copied();
        // The number where there is one, because a `.local` name only resolves
        // on a machine running a responder of its own, and the name otherwise —
        // which is still better than nothing and often works.
        let at = match address {
            Some(address) => address.to_string(),
            None => host.clone(),
        };
        found.push(Found {
            name: friendly(&instance.name),
            uri: uri_for(service, &at, instance.port, &instance.txt),
            host,
            address,
            port: instance.port,
            kind: service.kind,
            encrypted: service.encrypted,
            txt: instance.txt.clone(),
        });
    }
    found
}

fn entry<'a>(instances: &'a mut BTreeMap<String, Instance>, name: &str) -> &'a mut Instance {
    // Keyed by the lowercased name because DNS is not case sensitive and a
    // device answering twice may not use the same case both times; the name is
    // kept as written, because that is the one to show somebody.
    instances
        .entry(name.to_ascii_lowercase())
        .or_insert_with(|| Instance {
            name: name.to_string(),
            ..Instance::default()
        })
}

/// The service an instance belongs to, judged by the end of its own name.
fn service_of(instance_lower: &str, only: Option<Kind>) -> Option<&'static Service> {
    SERVICES.iter().find(|service| {
        wanted(service, only)
            && instance_lower
                .strip_suffix(service.advertised)
                .is_some_and(|head| head.ends_with('.'))
    })
}

/// The service by its own name, for the second opinion above.
fn service_named(advertised_lower: &str, only: Option<Kind>) -> Option<&'static Service> {
    SERVICES
        .iter()
        .find(|service| wanted(service, only) && service.advertised == advertised_lower)
}

/// An address a device could actually be at.
///
/// A printer that has not been given an address yet announces 0.0.0.0, and a
/// mangled record can hold anything at all. Offering either of them is offering
/// somebody a printer that cannot be reached.
fn usable(address: Ipv4Addr) -> bool {
    !(address.is_unspecified() || address.is_multicast() || address.is_broadcast())
}

/// The address a caller can hand straight to the printing or scanning code.
fn uri_for(service: &Service, at: &str, port: u16, txt: &[(String, String)]) -> String {
    // An address with colons in it is IPv6, and has to be written in brackets
    // or everything after its first colon reads as a port number.
    let at = if at.contains(':') {
        format!("[{at}]")
    } else {
        at.to_string()
    };
    let authority = if port == service.usual_port {
        at
    } else {
        format!("{at}:{port}")
    };

    // Where to send the job is the printer's to decide, and it says so in the
    // text record: `rp=ipp/print` on most machines, but `rp=printers/Office` on
    // a print server and `rp=ipp/printer1` on a multi-tray machine. Assuming
    // the usual path instead would print to the wrong queue, or to nothing.
    let path = txt
        .iter()
        .find(|(key, _)| !service.path_key.is_empty() && key == service.path_key)
        .map(|(_, value)| value.trim_matches('/'))
        .filter(|value| !value.is_empty())
        .unwrap_or(service.path);

    if path.is_empty() {
        format!("{}://{authority}", service.scheme)
    } else {
        format!("{}://{authority}/{path}", service.scheme)
    }
}

// ---------------------------------------------------------------------------
// One device, one entry
// ---------------------------------------------------------------------------

/// Fold the answers down to one entry per thing a caller could talk to.
///
/// One printer answers several times over. It is advertised as `_ipp` and again
/// as `_ipps`; a machine with a cable and Wi-Fi both plugged in answers on
/// each; and a print server republishes the printers behind it. Every one of
/// those arrives as a separate set of records, and a list showing the same
/// printer four times is a list somebody has to work out for themselves.
///
/// The thing they all have in common is the host and the port — the socket at
/// the far end — so that is what merges. What survives is the encrypted entry
/// where there is one, since a device offering encryption would rather be
/// spoken to that way, and [`Found::plain_uri`] is there for a caller that
/// cannot. The raw port 9100 service does *not* merge into the IPP one, because
/// it is a genuinely different way to talk to the machine on a different port,
/// and it sorts after IPP so the better one is offered first.
fn merge(found: Vec<Found>) -> Vec<Found> {
    let mut kept: Vec<Found> = Vec::new();
    for one in found {
        match kept
            .iter_mut()
            .find(|seen| seen.port == one.port && seen.host.eq_ignore_ascii_case(&one.host))
        {
            Some(seen) => absorb(seen, one),
            None => kept.push(one),
        }
    }
    // In a settled order, because a list that shuffles itself every time it is
    // refreshed is one nobody can click on.
    kept.sort_by(|one, two| {
        one.name
            .to_lowercase()
            .cmp(&two.name.to_lowercase())
            .then(one.port.cmp(&two.port))
            .then(one.host.to_lowercase().cmp(&two.host.to_lowercase()))
    });
    kept
}

/// Which of two entries for the same socket is the one to keep.
///
/// Encryption first, because that is the offer the device would rather have
/// taken up; then an address, because a numeric one reaches the machine from
/// anywhere and a `.local` name does not.
fn worth(found: &Found) -> u8 {
    (u8::from(found.encrypted) << 1) | u8::from(found.address.is_some())
}

fn absorb(kept: &mut Found, other: Found) {
    let (mut keep, spare) = if worth(&other) > worth(kept) {
        (other, kept.clone())
    } else {
        (kept.clone(), other)
    };
    // The loser is not thrown away whole: one advertisement often carries a
    // detail the other left out.
    for (key, value) in spare.txt {
        if !keep.txt.iter().any(|(seen, _)| seen == &key) {
            keep.txt.push((key, value));
        }
    }
    if keep.name.is_empty() {
        keep.name = spare.name;
    }
    if keep.address.is_none() {
        keep.address = spare.address;
    }
    *kept = keep;
}

#[cfg(test)]
mod tests;
