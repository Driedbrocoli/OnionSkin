//! Tests for finding a printer without being told where it is.
//!
//! Not one of these needs a network. The replies are built here byte by byte,
//! the way a printer builds them — compression pointers and all — because the
//! hard part of this module is reading a message somebody else wrote, and the
//! only honest way to test that is to write the message somebody else would
//! have sent.
//!
//! The builders below deliberately do not use the module's own [`encode_name`]:
//! a test that assembled its packets with the code under test would agree with
//! whatever that code happened to do.

use super::*;

// ---------------------------------------------------------------------------
// Building a reply by hand
// ---------------------------------------------------------------------------

/// A name written out in full, as a message that compresses nothing holds it.
fn labels(name: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    for label in name.split('.') {
        bytes.push(label.len() as u8);
        bytes.extend_from_slice(label.as_bytes());
    }
    bytes.push(0);
    bytes
}

/// The two bytes standing for "the name written at this offset".
fn pointer(at: usize) -> [u8; 2] {
    ((0xc000 | at) as u16).to_be_bytes()
}

/// The head of a reply holding `answers` records and nothing else.
fn header(answers: u16) -> Vec<u8> {
    // 0x8400: an answer, from the machine the name belongs to.
    let mut bytes = vec![0x00, 0x00, 0x84, 0x00];
    bytes.extend_from_slice(&0u16.to_be_bytes()); // no questions
    bytes.extend_from_slice(&answers.to_be_bytes());
    bytes.extend_from_slice(&0u16.to_be_bytes()); // nothing in authority
    bytes.extend_from_slice(&0u16.to_be_bytes()); // nothing extra
    bytes
}

/// One record: the name, then the type, class, time to live and the data.
fn record(bytes: &mut Vec<u8>, name: &[u8], kind: u16, data: &[u8]) {
    bytes.extend_from_slice(name);
    bytes.extend_from_slice(&kind.to_be_bytes());
    bytes.extend_from_slice(&0x8001u16.to_be_bytes()); // IN, with cache-flush
    bytes.extend_from_slice(&4500u32.to_be_bytes());
    bytes.extend_from_slice(&(data.len() as u16).to_be_bytes());
    bytes.extend_from_slice(data);
}

/// The strings of a text record, each with its length in front of it.
fn text(strings: &[&str]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for string in strings {
        bytes.push(string.len() as u8);
        bytes.extend_from_slice(string.as_bytes());
    }
    bytes
}

/// A whole reply about one device, compressed the way a real one is.
///
/// The pointer record's data ends with a pointer back to the record's own name,
/// the service and text records are named by a pointer to the name inside that
/// data, and the host in the service record ends with a pointer to the `local`
/// in the name at the top. Every one of those is what a printer actually sends,
/// and a parser that gets any of them wrong finds nothing at all.
fn reply(
    service_name: &str,
    instance: &str,
    host: &str,
    port: u16,
    strings: &[&str],
    address: Option<[u8; 4]>,
) -> Vec<u8> {
    let (name, after) = instance.split_once('.').expect("an instance and a service");
    assert_eq!(after, service_name, "the instance must be of that service");
    let (host_name, domain) = host.split_once('.').expect("a host and a domain");

    let mut bytes = header(3 + u16::from(address.is_some()));
    let service_at = bytes.len();
    let service = labels(service_name);
    // Where the domain starts inside that name. A label costs its length plus
    // one on the wire and its length plus a dot as text, so the two agree.
    let domain_at = service_at
        + service_name
            .strip_suffix(domain)
            .expect("the host and the service share a domain")
            .len();

    let mut named = vec![name.len() as u8];
    named.extend_from_slice(name.as_bytes());
    named.extend_from_slice(&pointer(service_at));
    // The instance name sits in the pointer record's data: after the record's
    // own name and the ten fixed bytes.
    let instance_at = bytes.len() + service.len() + 10;
    record(&mut bytes, &service, 12, &named);

    let mut where_it_is = vec![0, 0, 0, 0]; // priority and weight
    where_it_is.extend_from_slice(&port.to_be_bytes());
    where_it_is.push(host_name.len() as u8);
    where_it_is.extend_from_slice(host_name.as_bytes());
    where_it_is.extend_from_slice(&pointer(domain_at));
    // The host sits six bytes into that data.
    let host_at = bytes.len() + 2 + 10 + 6;
    record(&mut bytes, &pointer(instance_at), 33, &where_it_is);

    record(&mut bytes, &pointer(instance_at), 16, &text(strings));
    if let Some(address) = address {
        record(&mut bytes, &pointer(host_at), 1, &address);
    }
    bytes
}

/// The reply an ordinary IPP printer sends.
fn a_printer() -> Vec<u8> {
    reply(
        "_ipp._tcp.local",
        "Office._ipp._tcp.local",
        "HP1.local",
        631,
        &[
            "rp=ipp/print",
            "ty=HP LaserJet 400",
            "note=Second floor",
            "adminurl=http://HP1.local/#hId-pgAirPrint",
            "Color=T",
        ],
        Some([192, 168, 1, 5]),
    )
}

// ---------------------------------------------------------------------------
// Reading a message
// ---------------------------------------------------------------------------

#[test]
fn a_reply_comes_apart_into_the_records_it_holds() {
    let records = parse(&a_printer());
    assert_eq!(records.len(), 4, "{records:#?}");

    assert_eq!(
        records[0],
        Record::Pointer {
            service: "_ipp._tcp.local".to_string(),
            instance: "Office._ipp._tcp.local".to_string(),
        }
    );
    assert_eq!(
        records[1],
        Record::Service {
            instance: "Office._ipp._tcp.local".to_string(),
            host: "HP1.local".to_string(),
            port: 631,
        }
    );
    assert_eq!(
        records[3],
        Record::Address {
            host: "HP1.local".to_string(),
            address: Ipv4Addr::new(192, 168, 1, 5),
        }
    );
}

#[test]
fn a_compressed_name_is_followed_to_the_one_it_stands_for() {
    // Every name in that reply but the first is two bytes pointing at another
    // name. Read without following them there is one record and no printer.
    let whole = a_printer();
    assert!(
        whole.windows(2).any(|pair| pair[0] == 0xc0),
        "the reply is not compressed, so this proves nothing"
    );

    let records = parse(&whole);
    let Record::Service { instance, host, .. } = &records[1] else {
        panic!("{records:#?}");
    };
    assert_eq!(instance, "Office._ipp._tcp.local");
    assert_eq!(host, "HP1.local", "the host was written as a pointer too");
}

#[test]
fn a_name_can_be_read_straight_out_of_the_middle_of_a_packet() {
    // The whole of the compression scheme in one line: a name at an offset.
    let mut bytes = header(0);
    let at = bytes.len();
    bytes.extend_from_slice(&labels("HP1.local"));
    let (name, after) = read_name(&bytes, at).unwrap();
    assert_eq!(name, "HP1.local");
    assert_eq!(after, bytes.len(), "the record carries on after the name");
}

#[test]
fn a_pointer_that_points_at_itself_does_not_hang() {
    // Anybody on the network can send this. Following it is a program that
    // stops for good on one packet.
    let mut bytes = header(1);
    let at = bytes.len();
    bytes.extend_from_slice(&pointer(at)); // a name that is only itself
    bytes.extend_from_slice(&12u16.to_be_bytes());
    bytes.extend_from_slice(&1u16.to_be_bytes());
    bytes.extend_from_slice(&4500u32.to_be_bytes());
    bytes.extend_from_slice(&0u16.to_be_bytes());

    let started = Instant::now();
    assert!(read_name(&bytes, at).is_none());
    assert!(parse(&bytes).is_empty());
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "it should give up at once"
    );
}

#[test]
fn two_pointers_pointing_at_each_other_do_not_hang_either() {
    let mut bytes = header(0);
    let first = bytes.len();
    let second = first + 2;
    bytes.extend_from_slice(&pointer(second));
    bytes.extend_from_slice(&pointer(first));

    let started = Instant::now();
    assert!(read_name(&bytes, first).is_none());
    assert!(read_name(&bytes, second).is_none());
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn a_pointer_into_nowhere_is_refused() {
    let mut bytes = header(0);
    let at = bytes.len();
    bytes.extend_from_slice(&pointer(9000)); // past the end of everything
    assert!(read_name(&bytes, at).is_none());
}

#[test]
fn a_label_that_runs_off_the_end_of_the_packet_is_refused() {
    let mut bytes = header(0);
    let at = bytes.len();
    bytes.push(40); // forty bytes of label, and four bytes of packet
    bytes.extend_from_slice(b"HP1.");
    assert!(read_name(&bytes, at).is_none());
}

#[test]
fn a_packet_that_stops_early_is_read_as_far_as_it_goes() {
    // Which is what a dropped fragment, a short read or a printer that gave up
    // halfway through composing a reply all look like.
    let whole = a_printer();
    let started = Instant::now();
    for length in 0..whole.len() {
        let records = parse(&whole[..length]);
        assert!(
            records.len() < 4,
            "{length} bytes cannot hold the whole reply: {records:#?}"
        );
    }
    assert_eq!(parse(&whole).len(), 4, "and the whole thing still reads");
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn a_record_claiming_more_data_than_arrived_is_not_believed() {
    let mut bytes = header(1);
    bytes.extend_from_slice(&labels("_ipp._tcp.local"));
    bytes.extend_from_slice(&12u16.to_be_bytes());
    bytes.extend_from_slice(&1u16.to_be_bytes());
    bytes.extend_from_slice(&4500u32.to_be_bytes());
    bytes.extend_from_slice(&4000u16.to_be_bytes()); // four thousand bytes
    bytes.extend_from_slice(b"nine"); // of which four arrived
    assert!(parse(&bytes).is_empty());
}

#[test]
fn a_reply_with_nothing_in_it_gives_nothing() {
    assert!(parse(&[]).is_empty());
    assert!(parse(&[0x00, 0x00]).is_empty());
    assert!(parse(&header(0)).is_empty());
    assert!(parse(b"HTTP/1.1 200 OK\r\n\r\n").is_empty());
}

#[test]
fn a_question_arriving_here_is_read_past_rather_than_taken_for_an_answer() {
    // A responder repeats the question it is answering. The records after it
    // are the point, and a parser that does not skip the question reads the
    // question as a record and then reads rubbish.
    let mut bytes = header(1);
    bytes[4..6].copy_from_slice(&1u16.to_be_bytes()); // one question
    let service_at = bytes.len();
    bytes.extend_from_slice(&labels("_ipp._tcp.local"));
    bytes.extend_from_slice(&12u16.to_be_bytes());
    bytes.extend_from_slice(&1u16.to_be_bytes());

    let mut named = vec![6];
    named.extend_from_slice(b"Office");
    named.extend_from_slice(&pointer(service_at));
    record(&mut bytes, &pointer(service_at), 12, &named);

    let records = parse(&bytes);
    assert_eq!(records.len(), 1, "{records:#?}");
    assert!(matches!(&records[0], Record::Pointer { instance, .. }
        if instance == "Office._ipp._tcp.local"));
}

#[test]
fn a_goodbye_is_not_a_device() {
    // A record with no time to live means the machine is going away. Listing
    // it offers somebody a printer that has just been switched off.
    let mut bytes = header(1);
    let service = labels("_ipp._tcp.local");
    let mut named = vec![6];
    named.extend_from_slice(b"Office");
    named.extend_from_slice(&pointer(12));
    bytes.extend_from_slice(&service);
    bytes.extend_from_slice(&12u16.to_be_bytes());
    bytes.extend_from_slice(&1u16.to_be_bytes());
    bytes.extend_from_slice(&0u32.to_be_bytes()); // no time to live at all
    bytes.extend_from_slice(&(named.len() as u16).to_be_bytes());
    bytes.extend_from_slice(&named);

    assert!(parse(&bytes).is_empty());
}

// ---------------------------------------------------------------------------
// Text records
// ---------------------------------------------------------------------------

#[test]
fn a_text_record_is_a_list_of_strings_rather_than_one() {
    // A printer sends a dozen, each with its own length byte, and reading the
    // record as a single string gets the first key and loses everything after.
    let pairs = text_pairs(&text(&[
        "rp=ipp/print",
        "ty=Brother HL-L2350DW",
        "pdl=application/pdf",
    ]));
    assert_eq!(
        pairs,
        vec![
            ("rp".to_string(), "ipp/print".to_string()),
            ("ty".to_string(), "Brother HL-L2350DW".to_string()),
            ("pdl".to_string(), "application/pdf".to_string()),
        ]
    );
}

#[test]
fn a_key_on_its_own_is_present_rather_than_missing() {
    let pairs = text_pairs(&text(&["air=none", "mopria-certified", "Color=T"]));
    assert_eq!(pairs[1], ("mopria-certified".to_string(), String::new()));
    // And the key is lowercased, because half the printers on the market
    // disagree about the case and none of them mean anything by it.
    assert_eq!(pairs[2].0, "color");
}

#[test]
fn only_the_first_equals_sign_separates_a_key_from_its_value() {
    let pairs = text_pairs(&text(&["adminurl=http://HP1.local/?a=b"]));
    assert_eq!(pairs[0].1, "http://HP1.local/?a=b");
}

#[test]
fn a_text_record_that_lies_about_its_lengths_is_read_as_far_as_it_goes() {
    assert!(text_pairs(&[40, b'r', b'p']).is_empty());
    assert!(text_pairs(&[0, 0, 0]).is_empty());
    // The good string before the bad one is still true.
    let mut data = text(&["rp=ipp/print"]);
    data.push(40);
    data.extend_from_slice(b"ty=");
    assert_eq!(text_pairs(&data).len(), 1);
}

// ---------------------------------------------------------------------------
// Names, written and read back
// ---------------------------------------------------------------------------

#[test]
fn a_name_written_out_reads_back_as_itself() {
    for name in [
        "_ipp._tcp.local",
        "HP1.local",
        "Office._ipp._tcp.local",
        "a",
    ] {
        let mut bytes = Vec::new();
        encode_name(&mut bytes, name);
        let (read, after) = read_name(&bytes, 0).unwrap();
        assert_eq!(read, name);
        assert_eq!(after, bytes.len());
    }
}

#[test]
fn a_name_is_written_as_a_length_and_then_the_letters() {
    // Spelled out, because everything else here would still pass if this
    // module and its tests agreed on a format nothing else speaks.
    let mut bytes = Vec::new();
    encode_name(&mut bytes, "_ipp._tcp.local");
    assert_eq!(bytes, b"\x04_ipp\x04_tcp\x05local\x00");
}

#[test]
fn a_dot_in_a_printers_name_is_not_the_dot_between_two_names() {
    // "Reception. Ground floor" is a perfectly ordinary thing to call a
    // printer. Joining the labels with dots and saying nothing would make that
    // name indistinguishable from a printer called "Reception" in a domain
    // called " Ground floor".
    let mut bytes = header(1);
    let service_at = bytes.len();
    let awkward = "Reception. Ground floor";
    let mut named = vec![awkward.len() as u8];
    named.extend_from_slice(awkward.as_bytes());
    named.extend_from_slice(&pointer(service_at));
    record(&mut bytes, &labels("_ipp._tcp.local"), 12, &named);

    let records = parse(&bytes);
    let Record::Pointer { instance, .. } = &records[0] else {
        panic!("{records:#?}");
    };
    assert_eq!(instance, "Reception\\. Ground floor._ipp._tcp.local");
    assert_eq!(friendly(instance), awkward);
}

#[test]
fn a_name_in_somebody_elses_alphabet_stays_readable() {
    // Escaping every byte above ASCII would turn a name somebody chose into a
    // row of numbers, which is worse than useless in a list they have to pick
    // from.
    let mut name = String::new();
    push_label(&mut name, "Bürodrucker".as_bytes());
    assert_eq!(name, "Bürodrucker");
    assert_eq!(friendly("Bürodrucker._ipp._tcp.local"), "Bürodrucker");
}

#[test]
fn the_numeric_escapes_other_tools_write_are_understood() {
    // Avahi writes a space as \032. Onionskin does not, but a name may reach
    // this from somewhere other than the wire.
    assert_eq!(friendly("Front\\032Desk._ipp._tcp.local"), "Front Desk");
    assert_eq!(
        split_labels("a\\.b.c"),
        vec!["a.b".to_string(), "c".to_string()]
    );
    assert_eq!(split_labels(""), Vec::<String>::new());
    assert_eq!(friendly(""), "");
}

// ---------------------------------------------------------------------------
// Records into devices
// ---------------------------------------------------------------------------

#[test]
fn a_reply_becomes_a_printer_with_an_address_to_print_to() {
    let found = assemble(&parse(&a_printer()), None);
    assert_eq!(found.len(), 1, "{found:#?}");
    let printer = &found[0];

    assert_eq!(printer.name, "Office");
    assert_eq!(printer.host, "HP1.local");
    assert_eq!(printer.address, Some(Ipv4Addr::new(192, 168, 1, 5)));
    assert_eq!(printer.port, 631);
    assert_eq!(printer.kind, Kind::Printer);
    assert!(!printer.encrypted);
    assert_eq!(printer.model(), Some("HP LaserJet 400"));
    assert_eq!(printer.location(), Some("Second floor"));
    // The number rather than the name: a .local name resolves only on a
    // machine running a responder of its own, and the number works anywhere.
    assert_eq!(printer.uri, "ipp://192.168.1.5/ipp/print");
}

#[test]
fn the_printer_decides_where_the_job_goes_rather_than_this_program() {
    // A print server publishes a queue per printer, and assuming the usual
    // path would send every job to the wrong one — or to nothing.
    let found = assemble(
        &parse(&reply(
            "_ipp._tcp.local",
            "Upstairs._ipp._tcp.local",
            "server.local",
            631,
            &["rp=printers/Upstairs"],
            Some([10, 0, 0, 9]),
        )),
        None,
    );
    assert_eq!(found[0].uri, "ipp://10.0.0.9/printers/Upstairs");
}

#[test]
fn an_unusual_port_is_written_into_the_address_and_the_usual_one_is_not() {
    let found = assemble(
        &parse(&reply(
            "_ipp._tcp.local",
            "Odd._ipp._tcp.local",
            "odd.local",
            8631,
            &["rp=ipp/print"],
            Some([10, 0, 0, 3]),
        )),
        None,
    );
    assert_eq!(found[0].uri, "ipp://10.0.0.3:8631/ipp/print");
}

#[test]
fn a_scanner_is_offered_at_the_address_escl_lives_at() {
    let found = assemble(
        &parse(&reply(
            "_uscan._tcp.local",
            "Canon TS8350._uscan._tcp.local",
            "canon.local",
            80,
            &["rs=eSCL", "ty=Canon PIXMA TS8350", "uuid=1c983b40-1234"],
            Some([192, 168, 1, 22]),
        )),
        None,
    );
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].kind, Kind::Scanner);
    assert_eq!(found[0].uri, "http://192.168.1.22/eSCL");
    assert_eq!(found[0].get("uuid"), Some("1c983b40-1234"));
}

#[test]
fn a_scanner_that_puts_escl_somewhere_else_is_taken_at_its_word() {
    let found = assemble(
        &parse(&reply(
            "_uscan._tcp.local",
            "Epson._uscan._tcp.local",
            "epson.local",
            8080,
            &["rs=/eSCL/scan/"],
            Some([192, 168, 1, 30]),
        )),
        None,
    );
    assert_eq!(found[0].uri, "http://192.168.1.30:8080/eSCL/scan");
}

#[test]
fn a_raw_port_9100_printer_is_offered_as_the_socket_it_is() {
    let found = assemble(
        &parse(&reply(
            "_pdl-datastream._tcp.local",
            "Old Brother._pdl-datastream._tcp.local",
            "brother.local",
            9100,
            &["ty=Brother HL-5240"],
            Some([192, 168, 1, 40]),
        )),
        None,
    );
    assert_eq!(found[0].kind, Kind::Printer);
    assert_eq!(found[0].uri, "socket://192.168.1.40");
    assert_eq!(found[0].port, 9100);
}

#[test]
fn a_device_that_gave_no_address_is_still_offered_by_name() {
    // Half a reply is better than none: the name usually resolves on the
    // machine that asked, because that machine is on the same link.
    let found = assemble(
        &parse(&reply(
            "_ipp._tcp.local",
            "Quiet._ipp._tcp.local",
            "quiet.local",
            631,
            &["rp=ipp/print"],
            None,
        )),
        None,
    );
    assert_eq!(found[0].address, None);
    assert_eq!(found[0].uri, "ipp://quiet.local/ipp/print");
}

#[test]
fn a_name_with_nothing_behind_it_is_not_offered() {
    // A pointer on its own says a printer exists and not where it is. Listing
    // it gives somebody a printer they cannot print to.
    let mut bytes = header(1);
    let service_at = bytes.len();
    let mut named = vec![6];
    named.extend_from_slice(b"Silent");
    named.extend_from_slice(&pointer(service_at));
    record(&mut bytes, &labels("_ipp._tcp.local"), 12, &named);

    let records = parse(&bytes);
    assert_eq!(records.len(), 1, "the pointer is there");
    assert!(assemble(&records, None).is_empty(), "but not the printer");
    // And it is exactly what the second round of questions is for.
    assert_eq!(
        chase(&records, None).len(),
        2,
        "one for the host, one for the details"
    );
}

#[test]
fn an_answer_about_something_that_is_not_a_printer_is_ignored() {
    // Responders volunteer everything they know. A television is not a printer.
    let found = assemble(
        &parse(&reply(
            "_airplay._tcp.local",
            "Living Room._airplay._tcp.local",
            "appletv.local",
            7000,
            &["model=AppleTV"],
            Some([192, 168, 1, 60]),
        )),
        None,
    );
    assert!(found.is_empty(), "{found:#?}");
}

#[test]
fn a_service_name_only_matches_on_a_whole_label() {
    assert!(service_of("office._ipp._tcp.local", None).is_some());
    assert!(service_of("office._ipps._tcp.local", None).is_some());
    // Not a printer called "notipp" in some other domain.
    assert!(service_of("not_ipp._tcp.local", None).is_none());
    assert!(service_of("_ipp._tcp.local", None).is_none());
}

#[test]
fn asking_only_for_printers_leaves_the_scanners_out() {
    let mut records = parse(&a_printer());
    records.extend(parse(&reply(
        "_uscan._tcp.local",
        "Canon._uscan._tcp.local",
        "canon.local",
        80,
        &["rs=eSCL"],
        Some([192, 168, 1, 22]),
    )));

    assert_eq!(assemble(&records, None).len(), 2);
    let printers = assemble(&records, Some(Kind::Printer));
    assert_eq!(printers.len(), 1);
    assert_eq!(printers[0].kind, Kind::Printer);
    assert_eq!(assemble(&records, Some(Kind::Scanner)).len(), 1);
}

#[test]
fn the_case_a_device_writes_its_name_in_makes_no_difference() {
    // DNS names are not case sensitive and firmware is inconsistent about it.
    // A printer answering in capitals is the same printer.
    let found = assemble(
        &parse(&reply(
            "_IPP._TCP.LOCAL",
            "Shouty._IPP._TCP.LOCAL",
            "SHOUTY.LOCAL",
            631,
            &["RP=ipp/print"],
            Some([192, 168, 1, 7]),
        )),
        None,
    );
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].name, "Shouty");
    assert_eq!(found[0].uri, "ipp://192.168.1.7/ipp/print");
}

#[test]
fn an_address_no_device_could_be_at_is_ignored() {
    // A printer that has not been given an address yet announces 0.0.0.0.
    let found = assemble(
        &parse(&reply(
            "_ipp._tcp.local",
            "New._ipp._tcp.local",
            "new.local",
            631,
            &["rp=ipp/print"],
            Some([0, 0, 0, 0]),
        )),
        None,
    );
    assert_eq!(found[0].address, None);
    assert_eq!(found[0].uri, "ipp://new.local/ipp/print");
}

// ---------------------------------------------------------------------------
// One device, one entry
// ---------------------------------------------------------------------------

/// An entry as `assemble` would have made it.
fn entry_for(service: &str, name: &str, host: &str, port: u16, address: Option<[u8; 4]>) -> Found {
    let instance = format!("{name}.{service}");
    let mut found = assemble(
        &parse(&reply(
            service,
            &instance,
            host,
            port,
            &["rp=ipp/print", "ty=HP LaserJet 400"],
            address,
        )),
        None,
    );
    assert_eq!(found.len(), 1, "{found:#?}");
    found.remove(0)
}

/// The same printer as ever, advertised under whichever service is asked for.
fn office(service: &str) -> Found {
    entry_for(service, "Office", "HP1.local", 631, Some([192, 168, 1, 5]))
}

#[test]
fn the_same_printer_answering_twice_is_listed_once() {
    // A machine with a cable and Wi-Fi both plugged in answers on each, and a
    // list with the same printer in it twice is one somebody has to work out
    // for themselves.
    let heard = vec![office("_ipp._tcp.local"), office("_ipp._tcp.local")];
    assert_eq!(merge(heard).len(), 1);
}

#[test]
fn the_encrypted_advertisement_is_the_one_that_survives() {
    let plain = office("_ipp._tcp.local");
    let secure = office("_ipps._tcp.local");

    for pair in [
        vec![plain.clone(), secure.clone()],
        vec![secure.clone(), plain.clone()],
    ] {
        let merged = merge(pair);
        assert_eq!(merged.len(), 1, "{merged:#?}");
        assert!(merged[0].encrypted);
        assert_eq!(merged[0].uri, "ipps://192.168.1.5/ipp/print");
        // And a caller that cannot speak it has somewhere to go.
        assert_eq!(merged[0].plain_uri(), "ipp://192.168.1.5/ipp/print");
    }
}

#[test]
fn what_one_advertisement_left_out_the_other_fills_in() {
    let mut plain = office("_ipp._tcp.local");
    plain
        .txt
        .push(("pdl".to_string(), "application/pdf".to_string()));

    let merged = merge(vec![office("_ipps._tcp.local"), plain]);
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].get("pdl"), Some("application/pdf"));
    assert_eq!(merged[0].get("ty"), Some("HP LaserJet 400"));
}

#[test]
fn a_different_port_is_a_different_way_in_and_is_kept() {
    // The raw socket on 9100 is not the IPP service on 631 — it is another way
    // to talk to the same box, and only one of them takes a page size.
    let raw = entry_for(
        "_pdl-datastream._tcp.local",
        "Office",
        "HP1.local",
        9100,
        Some([192, 168, 1, 5]),
    );
    let merged = merge(vec![raw, office("_ipp._tcp.local")]);
    assert_eq!(merged.len(), 2);
    // The better one first: same name, so the port settles it.
    assert_eq!(merged[0].port, 631);
    assert_eq!(merged[1].port, 9100);
}

#[test]
fn two_different_printers_are_two_printers() {
    let two = entry_for(
        "_ipp._tcp.local",
        "Upstairs",
        "HP2.local",
        631,
        Some([192, 168, 1, 6]),
    );
    assert_eq!(merge(vec![office("_ipp._tcp.local"), two]).len(), 2);
}

#[test]
fn the_list_comes_back_in_the_same_order_every_time() {
    // A list that shuffles itself between refreshes is one nobody can click on.
    let printers = || {
        vec![
            entry_for("_ipp._tcp.local", "Upstairs", "b.local", 631, None),
            entry_for("_ipp._tcp.local", "Office", "a.local", 631, None),
            entry_for("_ipp._tcp.local", "attic", "c.local", 631, None),
        ]
    };
    let names: Vec<String> = merge(printers()).into_iter().map(|one| one.name).collect();
    assert_eq!(names, vec!["attic", "Office", "Upstairs"]);

    let mut backwards = printers();
    backwards.reverse();
    let again: Vec<String> = merge(backwards).into_iter().map(|one| one.name).collect();
    assert_eq!(names, again);
}

// ---------------------------------------------------------------------------
// The question on the wire
// ---------------------------------------------------------------------------

#[test]
fn a_question_is_the_bytes_a_responder_is_waiting_for() {
    let bytes = question("_ipp._tcp.local", record_type::PTR);

    assert_eq!(&bytes[2..4], &[0x00, 0x00], "a query carries no flags");
    assert_eq!(&bytes[4..6], &1u16.to_be_bytes(), "one question");
    assert_eq!(&bytes[6..12], &[0, 0, 0, 0, 0, 0], "and nothing else");
    assert_eq!(&bytes[12..29], b"\x04_ipp\x04_tcp\x05local\x00");
    assert_eq!(&bytes[29..31], &12u16.to_be_bytes(), "asking for a pointer");
    // Class IN, with the top bit asking for the answer to come straight back
    // here rather than to every machine on the link.
    assert_eq!(&bytes[31..33], &0x8001u16.to_be_bytes());
    assert_eq!(bytes.len(), 33);
}

#[test]
fn every_service_worth_asking_about_is_asked_about() {
    let advertised: Vec<&str> = SERVICES.iter().map(|service| service.advertised).collect();
    for expected in [
        "_ipp._tcp.local",
        "_ipps._tcp.local",
        "_pdl-datastream._tcp.local",
        "_uscan._tcp.local",
        "_uscans._tcp.local",
        "_scanner._tcp.local",
    ] {
        assert!(
            advertised.contains(&expected),
            "{expected} is not asked for"
        );
    }
}

#[test]
fn a_device_that_answered_in_full_is_not_asked_again() {
    // The second round exists for the ones that answer with a name and nothing
    // else. Asking a printer that already told us everything is a packet on
    // somebody's network for no reason.
    assert!(chase(&parse(&a_printer()), None).is_empty());
}

// ---------------------------------------------------------------------------
// Whatever anybody sends
// ---------------------------------------------------------------------------

#[test]
fn nothing_anybody_sends_can_make_this_panic_or_hang() {
    // The socket this reads from is open to the whole link, so "the packet is
    // hostile" is not a hypothetical. Every one of these is fed through the
    // parser and the result thrown away: the test is that it comes back.
    let started = Instant::now();

    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for _ in 0..4000 {
        let length = (next() % 400) as usize;
        let bytes: Vec<u8> = (0..length).map(|_| next() as u8).collect();
        let _ = parse(&bytes);
    }

    // And a real reply with one byte of it changed, everywhere, to each of the
    // values that mean something to the format: a pointer, a length, and the
    // end of a name.
    let whole = a_printer();
    for at in 0..whole.len() {
        for value in [0x00u8, 0x3f, 0xc0, 0xff] {
            let mut damaged = whole.clone();
            damaged[at] = value;
            let _ = parse(&damaged);
            let _ = assemble(&parse(&damaged), None);
        }
    }
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "something took far longer than reading a packet should"
    );
}

#[test]
fn looking_where_there_is_nothing_to_find_gives_an_empty_list_and_not_a_complaint() {
    // The only test here that opens a socket, and it asserts nothing about what
    // comes back: the machine running it may have a printer on the desk, or no
    // network at all, and both are a pass. What is being checked is that
    // neither of them is an error, and that the wait is bounded whatever the
    // caller asks for.
    let started = Instant::now();
    let found = find(Duration::from_millis(0));
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "a wait of nothing at all should not take five seconds"
    );
    for one in &found {
        assert!(!one.uri.is_empty(), "{one:#?}");
        assert!(!one.host.is_empty(), "{one:#?}");
    }
}
