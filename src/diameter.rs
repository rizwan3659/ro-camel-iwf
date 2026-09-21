//! Bounded Diameter framing. Application-specific AVP policy belongs in adapters.
use anyhow::{Result, ensure};

pub const MAX_FRAME: usize = 64 * 1024;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Avp {
    pub code: u32,
    pub flags: u8,
    pub vendor: Option<u32>,
    pub data: Vec<u8>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub flags: u8,
    pub command: u32,
    pub application: u32,
    pub hop: u32,
    pub end: u32,
    pub avps: Vec<Avp>,
}
fn u24(b: &[u8]) -> usize {
    ((b[0] as usize) << 16) | ((b[1] as usize) << 8) | b[2] as usize
}
fn u32be(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}
fn put24(out: &mut Vec<u8>, n: usize) {
    out.extend_from_slice(&[(n >> 16) as u8, (n >> 8) as u8, n as u8]);
}

impl Avp {
    fn octets(code: u32, data: impl Into<Vec<u8>>) -> Self {
        Self {
            code,
            flags: 0x40,
            vendor: None,
            data: data.into(),
        }
    }
    fn number(code: u32, value: u32) -> Self {
        Self::octets(code, value.to_be_bytes().to_vec())
    }
    fn grouped(code: u32, avps: Vec<Avp>) -> Result<Self> {
        let bytes = Message {
            flags: 0,
            command: 0,
            application: 0,
            hop: 0,
            end: 0,
            avps,
        }
        .encode()?;
        Ok(Self::octets(code, bytes[20..].to_vec()))
    }
}

/// A minimal single-service, time-based CCR. Not a complete 3GPP Ro profile.
/// Caller supplies globally unique session IDs, peer identities and an agreed service context.
pub fn credit_request(
    ccr: &crate::charging::Ccr,
    host: &str,
    realm: &str,
    destination: &str,
    context: &str,
    hop: u32,
    end: u32,
) -> Result<Message> {
    ensure!(
        [host, realm, destination, context]
            .iter()
            .all(|v| !v.is_empty() && v.len() <= 255),
        "invalid identity"
    );
    let mut avps = vec![
        Avp::octets(263, ccr.session_id.as_bytes()),
        Avp::octets(264, host.as_bytes()),
        Avp::octets(296, realm.as_bytes()),
        Avp::octets(283, destination.as_bytes()),
        Avp::number(258, 4),
        Avp::octets(461, context.as_bytes()),
        Avp::number(416, ccr.kind as u32),
        Avp::number(415, ccr.number),
        Avp::grouped(
            443,
            vec![
                Avp::number(450, 0),
                Avp::octets(444, ccr.subscriber.as_bytes()),
            ],
        )?,
    ];
    if ccr.kind != crate::charging::RequestType::Initial {
        avps.push(Avp::grouped(446, vec![Avp::number(420, ccr.used_seconds)])?);
    }
    if ccr.kind != crate::charging::RequestType::Termination {
        avps.push(Avp::grouped(
            437,
            vec![Avp::number(420, ccr.requested_seconds)],
        )?);
    } else {
        avps.push(Avp::number(295, 1));
    }
    Ok(Message {
        flags: 0xc0,
        command: 272,
        application: 4,
        hop,
        end,
        avps,
    })
}

impl Message {
    fn group(avp: &Avp) -> Result<Self> {
        ensure!(avp.data.len() <= MAX_FRAME - 20, "group too large");
        let mut bytes = vec![1, 0, 0, 0, 0, 0, 0, 0];
        bytes.resize(20, 0);
        bytes.extend_from_slice(&avp.data);
        let n = bytes.len();
        bytes[1..4].copy_from_slice(&[(n >> 16) as u8, (n >> 8) as u8, n as u8]);
        Self::decode(&bytes)
    }

    fn text(&self, code: u32) -> Result<String> {
        let a = self
            .unique(code)?
            .ok_or_else(|| anyhow::anyhow!("missing text AVP"))?;
        ensure!(
            !a.data.is_empty() && a.data.len() <= 255,
            "invalid text AVP"
        );
        Ok(std::str::from_utf8(&a.data)?.to_owned())
    }

    /// Strict simulator profile: one MSISDN, single-service seconds, no MSCC.
    pub fn imscap_request(&self) -> Result<crate::charging::Ccr> {
        use crate::charging::{Ccr, RequestType};
        ensure!(
            self.command == 272
                && self.application == 4
                && self.flags & 0x80 != 0
                && self.flags & 0x20 == 0,
            "not a CCR"
        );
        let allowed = [
            263, 264, 296, 283, 293, 258, 461, 416, 415, 443, 446, 437, 295,
        ];
        for a in &self.avps {
            ensure!(
                a.vendor.is_none() && allowed.contains(&a.code),
                "unsupported simulator AVP"
            );
            self.unique(a.code)?;
        }
        ensure!(self.integer(258)? == 4, "wrong application");
        for code in [264, 296, 283, 461] {
            self.text(code)?;
        }
        let kind = match self.integer(416)? {
            1 => RequestType::Initial,
            2 => RequestType::Update,
            3 => RequestType::Termination,
            _ => anyhow::bail!("unsupported request type"),
        };
        let sub = Self::group(
            self.unique(443)?
                .ok_or_else(|| anyhow::anyhow!("missing subscriber"))?,
        )?;
        ensure!(
            sub.avps.len() == 2 && sub.integer(450)? == 0,
            "expected one MSISDN"
        );
        let units = |code| -> Result<u32> {
            let group = Self::group(
                self.unique(code)?
                    .ok_or_else(|| anyhow::anyhow!("missing units"))?,
            )?;
            ensure!(group.avps.len() == 1, "only CC-Time supported");
            group.integer(420)
        };
        let used_seconds = if kind == RequestType::Initial {
            ensure!(self.unique(446)?.is_none(), "initial usage unsupported");
            0
        } else {
            units(446)?
        };
        let requested_seconds = if kind == RequestType::Termination {
            ensure!(self.unique(437)?.is_none(), "termination requests units");
            self.integer(295)?;
            0
        } else {
            units(437)?
        };
        Ok(Ccr {
            session_id: self.text(263)?,
            subscriber: sub.text(444)?,
            number: self.integer(415)?,
            kind,
            used_seconds,
            requested_seconds,
        })
    }

    /// Encode an IMSCAP decision using this CCR's transport correlation identifiers.
    pub fn imscap_answer(
        &self,
        action: &crate::imscap::Action,
        host: &str,
        realm: &str,
    ) -> Result<Self> {
        let request = self.imscap_request()?;
        let crate::imscap::Action::Answer {
            number,
            kind,
            result,
            seconds,
        } = action
        else {
            anyhow::bail!("not an answer")
        };
        ensure!(
            *number == request.number && *kind == request.kind,
            "answer correlation mismatch"
        );
        ensure!(
            !host.is_empty() && host.len() <= 255 && !realm.is_empty() && realm.len() <= 255,
            "invalid origin"
        );
        let mut avps = vec![
            Avp::octets(263, request.session_id.into_bytes()),
            Avp::octets(264, host.as_bytes()),
            Avp::octets(296, realm.as_bytes()),
            Avp::number(258, 4),
            Avp::number(416, *kind as u32),
            Avp::number(415, *number),
            Avp::number(268, *result),
        ];
        if *seconds > 0 {
            avps.push(Avp::grouped(431, vec![Avp::number(420, *seconds)])?);
        }
        Ok(Self {
            flags: self.flags & 0x40,
            command: 272,
            application: 4,
            hop: self.hop,
            end: self.end,
            avps,
        })
    }

    fn unique(&self, code: u32) -> Result<Option<&Avp>> {
        let mut matches = self
            .avps
            .iter()
            .filter(|a| a.code == code && a.vendor.is_none());
        let first = matches.next();
        ensure!(matches.next().is_none(), "duplicate singleton AVP");
        Ok(first)
    }

    fn integer(&self, code: u32) -> Result<u32> {
        let a = self
            .unique(code)?
            .ok_or_else(|| anyhow::anyhow!("missing integer AVP"))?;
        ensure!(a.data.len() == 4, "invalid integer AVP");
        Ok(u32be(&a.data))
    }

    /// Validate the intentionally narrow CCA subset against the outstanding CCR.
    /// Unsupported multi-service, validity-time and final-unit policies are rejected.
    pub fn credit_answer(
        &self,
        request: &crate::charging::Ccr,
        hop: u32,
        end: u32,
    ) -> Result<crate::charging::Event> {
        ensure!(
            self.flags & 0x80 == 0 && self.command == 272 && self.application == 4,
            "not a credit-control answer"
        );
        ensure!(
            self.hop == hop && self.end == end,
            "Diameter identifiers do not match"
        );
        let id = self
            .unique(263)?
            .ok_or_else(|| anyhow::anyhow!("missing session id"))?;
        ensure!(id.data == request.session_id.as_bytes(), "session mismatch");
        ensure!(
            self.integer(258)? == 4
                && self.integer(415)? == request.number
                && self.integer(416)? == request.kind as u32,
            "application/request mismatch"
        );
        for code in [264, 296] {
            ensure!(
                self.unique(code)?.is_some_and(|v| !v.data.is_empty()),
                "missing origin identity"
            );
        }
        for code in [456, 430, 448] {
            ensure!(self.unique(code)?.is_none(), "unsupported charging policy");
        }
        let success = self.integer(268)? == 2001 && self.flags & 0x20 == 0;
        let granted = if let Some(gsu) = self.unique(431)? {
            ensure!(gsu.data.len() <= MAX_FRAME - 20, "grouped AVP too large");
            let mut frame = Message {
                flags: 0,
                command: 0,
                application: 0,
                hop: 0,
                end: 0,
                avps: vec![],
            }
            .encode()?;
            frame.extend_from_slice(&gsu.data);
            let length = frame.len();
            frame[1..4].copy_from_slice(&[(length >> 16) as u8, (length >> 8) as u8, length as u8]);
            let group = Message::decode(&frame)?;
            ensure!(
                group
                    .avps
                    .iter()
                    .all(|a| a.code == 420 && a.vendor.is_none()),
                "only time units supported"
            );
            group.integer(420)?
        } else {
            0
        };
        Ok(crate::charging::Event::Answer {
            number: request.number,
            success,
            granted,
        })
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        ensure!(
            (20..=MAX_FRAME).contains(&bytes.len()),
            "frame size out of range"
        );
        ensure!(
            bytes[0] == 1 && u24(&bytes[1..4]) == bytes.len(),
            "invalid Diameter header"
        );
        ensure!(bytes[4] & 0x0f == 0, "reserved command flags");
        let mut avps = Vec::new();
        let mut offset = 20;
        while offset < bytes.len() {
            ensure!(bytes.len() - offset >= 8, "truncated AVP header");
            let flags = bytes[offset + 4];
            ensure!(flags & 0x1f == 0, "reserved AVP flags");
            let header = if flags & 0x80 != 0 { 12 } else { 8 };
            let length = u24(&bytes[offset + 5..offset + 8]);
            ensure!(
                length >= header && length <= bytes.len() - offset,
                "invalid AVP length"
            );
            let padded = (length + 3) & !3;
            ensure!(padded <= bytes.len() - offset, "truncated AVP padding");
            ensure!(avps.len() < 256, "too many AVPs");
            avps.push(Avp {
                code: u32be(&bytes[offset..offset + 4]),
                flags,
                vendor: if header == 12 {
                    Some(u32be(&bytes[offset + 8..offset + 12]))
                } else {
                    None
                },
                data: bytes[offset + header..offset + length].to_vec(),
            });
            offset += padded;
        }
        Ok(Self {
            flags: bytes[4],
            command: u24(&bytes[5..8]) as u32,
            application: u32be(&bytes[8..12]),
            hop: u32be(&bytes[12..16]),
            end: u32be(&bytes[16..20]),
            avps,
        })
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        ensure!(
            self.command <= 0xffffff && self.flags & 0x0f == 0,
            "invalid command"
        );
        ensure!(self.avps.len() <= 256, "too many AVPs");
        let mut out = vec![1, 0, 0, 0, self.flags];
        put24(&mut out, self.command as usize);
        for n in [self.application, self.hop, self.end] {
            out.extend_from_slice(&n.to_be_bytes());
        }
        for a in &self.avps {
            ensure!(
                a.flags & 0x1f == 0 && (a.flags & 0x80 != 0) == a.vendor.is_some(),
                "invalid AVP flags"
            );
            let length = a
                .data
                .len()
                .checked_add(if a.vendor.is_some() { 12 } else { 8 })
                .ok_or_else(|| anyhow::anyhow!("AVP too large"))?;
            ensure!(
                length <= MAX_FRAME && out.len() + ((length + 3) & !3) <= MAX_FRAME,
                "frame too large"
            );
            out.extend_from_slice(&a.code.to_be_bytes());
            out.push(a.flags);
            put24(&mut out, length);
            if let Some(v) = a.vendor {
                out.extend_from_slice(&v.to_be_bytes());
            }
            out.extend_from_slice(&a.data);
            while out.len() % 4 != 0 {
                out.push(0);
            }
        }
        let len = out.len();
        out[1..4].copy_from_slice(&[(len >> 16) as u8, (len >> 8) as u8, len as u8]);
        Ok(out)
    }
}
