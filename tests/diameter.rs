use ro_camel_iwf::{
    charging::{Ccr, RequestType},
    diameter::{Avp, Message, credit_request},
};
fn request() -> Message {
    credit_request(
        &Ccr {
            session_id: "iwf.test;1".into(),
            subscriber: "12345".into(),
            number: 0,
            kind: RequestType::Initial,
            used_seconds: 0,
            requested_seconds: 60,
        },
        "iwf.test",
        "test",
        "ocs.test",
        "voice@test",
        123,
        456,
    )
    .unwrap()
}
#[test]
fn ccr_header_and_mandatory_identity_fields() {
    let m = request();
    let encoded = m.encode().unwrap();
    assert_eq!(&encoded[..1], &[1]);
    assert_eq!(&encoded[4..12], &[0xc0, 0, 1, 0x10, 0, 0, 0, 4]);
    assert_eq!(Message::decode(&encoded).unwrap(), m);
    for code in [263, 264, 296, 283, 258, 461, 416, 415, 443, 437] {
        assert_eq!(m.avps.iter().filter(|a| a.code == code).count(), 1);
    }
}
#[test]
fn reject_all_truncations_and_overlarge_frame() {
    let bytes = request().encode().unwrap();
    for n in 0..bytes.len() {
        assert!(Message::decode(&bytes[..n]).is_err());
    }
    assert!(Message::decode(&vec![0; 65537]).is_err());
}
#[test]
fn avp_padding_vendor_and_zero_length_rejection() {
    let m = Message {
        flags: 0,
        command: 272,
        application: 4,
        hop: 1,
        end: 2,
        avps: vec![Avp {
            code: 1,
            flags: 0xc0,
            vendor: Some(10415),
            data: vec![7, 8, 9],
        }],
    };
    let mut bytes = m.encode().unwrap();
    assert_eq!(bytes.len(), 36);
    assert_eq!(Message::decode(&bytes).unwrap(), m);
    bytes[25..28].fill(0);
    assert!(Message::decode(&bytes).is_err());
}
#[test]
fn arbitrary_malformed_inputs_never_panic() {
    let mut seed = 123u64;
    for len in 0..1024 {
        let mut bytes = vec![0; len];
        for b in &mut bytes {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            *b = (seed >> 32) as u8;
        }
        let _ = Message::decode(&bytes);
    }
}
#[test]
fn avp_payload_limit_checked_before_output_allocation() {
    let mut m = request();
    m.avps.push(Avp {
        code: 1,
        flags: 0,
        vendor: None,
        data: vec![0; 65536],
    });
    assert!(m.encode().is_err());
}

fn answer() -> (Message, Ccr) {
    let ccr = Ccr {
        session_id: "iwf.test;1".into(),
        subscriber: "12345".into(),
        number: 0,
        kind: RequestType::Initial,
        used_seconds: 0,
        requested_seconds: 60,
    };
    let mut message = request();
    message.flags = 0x40;
    message
        .avps
        .retain(|a| ![283, 461, 443, 437].contains(&a.code));
    message.avps.push(Avp {
        code: 268,
        flags: 0x40,
        vendor: None,
        data: 2001u32.to_be_bytes().to_vec(),
    });
    // CC-Time 420, M flag, AVP length 12, 60 seconds; grouped into GSU 431.
    message.avps.push(Avp {
        code: 431,
        flags: 0x40,
        vendor: None,
        data: vec![0, 0, 1, 164, 0x40, 0, 0, 12, 0, 0, 0, 60],
    });
    (message, ccr)
}

#[test]
fn cca_validates_wire_identifiers_and_extracts_time_grant() {
    let (message, ccr) = answer();
    let wire = message.encode().unwrap();
    let decoded = Message::decode(&wire).unwrap();
    assert_eq!(
        decoded.credit_answer(&ccr, 123, 456).unwrap(),
        ro_camel_iwf::charging::Event::Answer {
            number: 0,
            success: true,
            granted: 60
        }
    );
    assert!(decoded.credit_answer(&ccr, 124, 456).is_err());
    let mut other = ccr;
    other.session_id = "another".into();
    assert!(decoded.credit_answer(&other, 123, 456).is_err());
}

#[test]
fn duplicate_result_and_unsupported_policy_rejected() {
    let (mut message, ccr) = answer();
    message
        .avps
        .push(message.avps.iter().find(|a| a.code == 268).unwrap().clone());
    assert!(message.credit_answer(&ccr, 123, 456).is_err());
    message.avps.pop();
    message.avps.push(Avp {
        code: 430,
        flags: 0x40,
        vendor: None,
        data: vec![],
    });
    assert!(message.credit_answer(&ccr, 123, 456).is_err());
}

#[test]
fn malformed_granted_units_are_rejected() {
    let (mut message, ccr) = answer();
    message
        .avps
        .iter_mut()
        .find(|a| a.code == 431)
        .unwrap()
        .data = vec![1, 2, 3];
    assert!(message.credit_answer(&ccr, 123, 456).is_err());
}
