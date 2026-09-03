use jcode_dap::{
    DapError, Event, FrameDecoder, Message, Request, decode_message, encode_frame, encode_message,
};

#[test]
fn decodes_a_frame_split_at_every_byte_boundary() {
    let payload = br#"{"seq":1,"type":"event","event":"ready"}"#;
    let encoded = encode_frame(payload);
    for split in 0..=encoded.len() {
        let mut decoder = FrameDecoder::default();
        let first = decoder.push(&encoded[..split]).unwrap();
        let mut frames = first;
        frames.extend(decoder.push(&encoded[split..]).unwrap());
        assert_eq!(frames, vec![payload.to_vec()], "split at {split}");
        assert_eq!(decoder.buffered_bytes(), 0);
    }
}

#[test]
fn decodes_multiple_frames_and_partial_tail() {
    let mut decoder = FrameDecoder::default();
    let third = encode_frame(b"three");
    let input = [
        encode_frame(b"one"),
        encode_frame(b"two"),
        third[..third.len() - 2].to_vec(),
    ]
    .concat();
    assert_eq!(
        decoder.push(&input).unwrap(),
        vec![b"one".to_vec(), b"two".to_vec()]
    );
    assert_eq!(decoder.push(b"ee").unwrap(), vec![b"three".to_vec()]);
}

#[test]
fn decodes_many_frames_with_one_compaction_pass() {
    let expected = (0..4096)
        .map(|index| index.to_string().into_bytes())
        .collect::<Vec<_>>();
    let encoded = expected
        .iter()
        .flat_map(|payload| encode_frame(payload))
        .collect::<Vec<_>>();
    let mut decoder = FrameDecoder::default();
    assert_eq!(decoder.push(&encoded).unwrap(), expected);
    assert_eq!(decoder.buffered_bytes(), 0);
}

#[test]
fn permits_a_maximum_sized_header_while_its_delimiter_is_partial() {
    let mut decoder = FrameDecoder::new(17, 1);
    assert!(decoder.push(b"Content-Length: 1\r\n\r").unwrap().is_empty());
    assert_eq!(decoder.push(b"\nx").unwrap(), vec![b"x".to_vec()]);
}

#[test]
fn rejects_malformed_headers_and_lengths() {
    let cases = [
        b"Content-Type: x\r\n\r\n".as_slice(),
        b"Content-Length: 1\r\nContent-Length: 1\r\n\r\nx".as_slice(),
        b"Content-Length: -1\r\n\r\n".as_slice(),
        b"Content-Length: +1\r\n\r\nx".as_slice(),
        b"Content-Leng\xffth: 1\r\n\r\nx".as_slice(),
        b"bad-header\r\n\r\n".as_slice(),
        b"Content-Length: 999999999999999999999999999999999999\r\n\r\n".as_slice(),
    ];
    for case in cases {
        assert!(FrameDecoder::default().push(case).is_err());
    }
}

#[test]
fn accepts_case_insensitive_content_length_and_ignores_other_headers() {
    let frame = b"content-length: 2\r\nContent-Type: application/json\r\n\r\n{}";
    assert_eq!(
        FrameDecoder::default().push(frame).unwrap(),
        vec![b"{}".to_vec()]
    );
}

#[test]
fn enforces_header_and_payload_limits() {
    assert_eq!(
        FrameDecoder::new(8, 64).push(b"123456789").unwrap_err(),
        DapError::HeaderTooLarge { limit: 8 }
    );
    assert_eq!(
        FrameDecoder::new(64, 3)
            .push(b"Content-Length: 4\r\n\r\n")
            .unwrap_err(),
        DapError::PayloadTooLarge {
            observed: 4,
            limit: 3
        }
    );
}

#[test]
fn protocol_dispatches_by_type_and_validates_identifiers() {
    assert!(matches!(
        decode_message(br#"{"seq":1,"type":"request","command":"threads"}"#).unwrap(),
        Message::Request(_)
    ));
    assert!(matches!(
        decode_message(
            br#"{"seq":2,"type":"response","request_seq":1,"success":true,"command":"threads"}"#
        )
        .unwrap(),
        Message::Response(_)
    ));
    assert!(matches!(
        decode_message(br#"{"seq":3,"type":"event","event":"stopped"}"#).unwrap(),
        Message::Event(_)
    ));
    for payload in [
        br#"{"seq":0,"type":"event","event":"x"}"#.as_slice(),
        br#"{"seq":1,"type":"request","command":""}"#.as_slice(),
        br#"{"seq":1,"type":"response","request_seq":0,"success":true,"command":"x"}"#.as_slice(),
        br#"{"seq":1,"type":"event","event":""}"#.as_slice(),
        br#"{"seq":1,"type":"unknown"}"#.as_slice(),
        b"{".as_slice(),
    ] {
        assert!(
            decode_message(payload).is_err(),
            "payload={}",
            String::from_utf8_lossy(payload)
        );
    }
}

#[test]
fn message_encoding_round_trips_through_framing() {
    let event = Event::new(1, "initialized", None);
    let encoded = encode_message(&event).unwrap();
    let frame = FrameDecoder::default().push(&encoded).unwrap().remove(0);
    assert_eq!(decode_message(&frame).unwrap(), Message::Event(event));
    assert!(encode_message(&Request::new(0, "bad", None)).is_err());
}
