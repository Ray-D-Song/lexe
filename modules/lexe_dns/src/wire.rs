// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
//! Pure-Rust RFC 1035 / 2782 / 3403 / 6698 / 8659 DNS wire-format codec.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

pub const TYPE_A: u16 = 1;
pub const TYPE_NS: u16 = 2;
pub const TYPE_CNAME: u16 = 5;
pub const TYPE_SOA: u16 = 6;
pub const TYPE_PTR: u16 = 12;
pub const TYPE_MX: u16 = 15;
pub const TYPE_TXT: u16 = 16;
pub const TYPE_AAAA: u16 = 28;
pub const TYPE_SRV: u16 = 33;
pub const TYPE_NAPTR: u16 = 35;
pub const TYPE_TLSA: u16 = 52;
pub const TYPE_ANY: u16 = 255;
pub const TYPE_CAA: u16 = 257;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DnsWireError {
    NoData,
    FormErr,
    ServFail,
    NotFound,
    NotImp,
    Refused,
    BadName,
    BadResp,
    ConnRefused,
    Timeout,
    Cancelled,
}

impl DnsWireError {
    pub fn code(self) -> &'static str {
        match self {
            Self::NoData => "ENODATA",
            Self::FormErr => "EFORMERR",
            Self::ServFail => "ESERVFAIL",
            Self::NotFound => "ENOTFOUND",
            Self::NotImp => "ENOTIMP",
            Self::Refused => "EREFUSED",
            Self::BadName => "EBADNAME",
            Self::BadResp => "EBADRESP",
            Self::ConnRefused => "ECONNREFUSED",
            Self::Timeout => "ETIMEOUT",
            Self::Cancelled => "ECANCELLED",
        }
    }

    pub fn is_retryable(self) -> bool {
        matches!(self, Self::ServFail | Self::Timeout | Self::ConnRefused)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DnsRecord {
    A {
        address: Ipv4Addr,
        ttl: u32,
    },
    Aaaa {
        address: Ipv6Addr,
        ttl: u32,
    },
    Cname(String),
    Ns(String),
    Ptr(String),
    Mx {
        priority: u16,
        exchange: String,
    },
    Txt(Vec<String>),
    Srv {
        priority: u16,
        weight: u16,
        port: u16,
        name: String,
    },
    Soa {
        nsname: String,
        hostmaster: String,
        serial: u32,
        refresh: i32,
        retry: i32,
        expire: i32,
        minttl: u32,
    },
    Naptr {
        order: u16,
        preference: u16,
        flags: String,
        service: String,
        regexp: String,
        replacement: String,
    },
    Caa {
        critical: u8,
        tag: String,
        value: String,
    },
    Tlsa {
        cert_usage: u8,
        selector: u8,
        match_type: u8,
        data: Vec<u8>,
    },
}

impl DnsRecord {
    pub fn rtype(&self) -> u16 {
        match self {
            Self::A { .. } => TYPE_A,
            Self::Aaaa { .. } => TYPE_AAAA,
            Self::Cname(_) => TYPE_CNAME,
            Self::Ns(_) => TYPE_NS,
            Self::Ptr(_) => TYPE_PTR,
            Self::Mx { .. } => TYPE_MX,
            Self::Txt(_) => TYPE_TXT,
            Self::Srv { .. } => TYPE_SRV,
            Self::Soa { .. } => TYPE_SOA,
            Self::Naptr { .. } => TYPE_NAPTR,
            Self::Caa { .. } => TYPE_CAA,
            Self::Tlsa { .. } => TYPE_TLSA,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedPacket {
    pub truncated: bool,
    pub rcode: u8,
    pub answers: Vec<DnsRecord>,
}

/// Encode a single-question DNS query packet with RD=1.
pub fn encode_query(id: u16, hostname: &str, qtype: u16) -> Result<Vec<u8>, DnsWireError> {
    if hostname.is_empty() {
        return Err(DnsWireError::BadName);
    }

    let mut packet = Vec::with_capacity(64);
    packet.extend_from_slice(&id.to_be_bytes());
    packet.extend_from_slice(&[0x01, 0x00]); // QR=0, RD=1
    packet.extend_from_slice(&1u16.to_be_bytes()); // QDCOUNT = 1
    packet.extend_from_slice(&[0u8; 6]); // ANCOUNT, NSCOUNT, ARCOUNT = 0

    let qname_start = packet.len();
    let trimmed = hostname.strip_suffix('.').unwrap_or(hostname);
    if !trimmed.is_empty() {
        if trimmed.starts_with('.') || trimmed.contains("..") {
            return Err(DnsWireError::BadName);
        }
        for label in trimmed.split('.') {
            let bytes = label.as_bytes();
            if bytes.is_empty() || bytes.len() > 63 {
                return Err(DnsWireError::BadName);
            }
            packet.push(bytes.len() as u8);
            packet.extend_from_slice(bytes);
        }
    }
    packet.push(0); // Root terminating label

    if packet.len() - qname_start > 255 {
        return Err(DnsWireError::BadName);
    }

    packet.extend_from_slice(&qtype.to_be_bytes());
    packet.extend_from_slice(&1u16.to_be_bytes()); // QCLASS = IN
    Ok(packet)
}

/// Safely decode a DNS response packet matching `expected_id`.
pub fn decode_response(buf: &[u8], expected_id: u16) -> Result<DecodedPacket, DnsWireError> {
    if buf.len() < 12 {
        return Err(DnsWireError::BadResp);
    }
    let id = u16::from_be_bytes([buf[0], buf[1]]);
    if id != expected_id || (buf[2] & 0x80) == 0 {
        return Err(DnsWireError::BadResp);
    }

    let truncated = (buf[2] & 0x02) != 0;
    let rcode = buf[3] & 0x0f;
    if truncated {
        return Ok(DecodedPacket {
            truncated: true,
            rcode,
            answers: Vec::new(),
        });
    }

    if rcode != 0 {
        return Err(match rcode {
            1 => DnsWireError::FormErr,
            2 => DnsWireError::ServFail,
            3 => DnsWireError::NotFound,
            4 => DnsWireError::NotImp,
            5 => DnsWireError::Refused,
            _ => DnsWireError::BadResp,
        });
    }

    let qdcount = u16::from_be_bytes([buf[4], buf[5]]) as usize;
    let ancount = u16::from_be_bytes([buf[6], buf[7]]) as usize;

    let mut offset = 12usize;
    for _ in 0..qdcount {
        let (_, next) = read_name(buf, offset)?;
        if next + 4 > buf.len() {
            return Err(DnsWireError::BadResp);
        }
        offset = next + 4;
    }

    let mut answers = Vec::with_capacity(ancount);
    for _ in 0..ancount {
        let (_, next) = read_name(buf, offset)?;
        if next + 10 > buf.len() {
            return Err(DnsWireError::BadResp);
        }
        let rtype = u16::from_be_bytes([buf[next], buf[next + 1]]);
        let rclass = u16::from_be_bytes([buf[next + 2], buf[next + 3]]);
        let ttl = u32::from_be_bytes([buf[next + 4], buf[next + 5], buf[next + 6], buf[next + 7]]);
        let rdlen = u16::from_be_bytes([buf[next + 8], buf[next + 9]]) as usize;
        let rdata_start = next + 10;
        let rdata_end = rdata_start
            .checked_add(rdlen)
            .ok_or(DnsWireError::BadResp)?;
        if rdata_end > buf.len() {
            return Err(DnsWireError::BadResp);
        }
        offset = rdata_end;

        if rclass != 1 {
            continue;
        }

        if let Some(rec) = decode_rdata(buf, rdata_start, rdata_end, rtype, ttl)? {
            answers.push(rec);
        }
    }

    Ok(DecodedPacket {
        truncated: false,
        rcode,
        answers,
    })
}

fn decode_rdata(
    buf: &[u8],
    start: usize,
    end: usize,
    rtype: u16,
    ttl: u32,
) -> Result<Option<DnsRecord>, DnsWireError> {
    let rdlen = end - start;
    match rtype {
        TYPE_A => {
            if rdlen != 4 {
                return Err(DnsWireError::BadResp);
            }
            Ok(Some(DnsRecord::A {
                address: Ipv4Addr::new(buf[start], buf[start + 1], buf[start + 2], buf[start + 3]),
                ttl,
            }))
        },
        TYPE_AAAA => {
            if rdlen != 16 {
                return Err(DnsWireError::BadResp);
            }
            let octets: [u8; 16] = buf[start..end]
                .try_into()
                .map_err(|_| DnsWireError::BadResp)?;
            Ok(Some(DnsRecord::Aaaa {
                address: Ipv6Addr::from(octets),
                ttl,
            }))
        },
        TYPE_CNAME => {
            let (name, next) = read_name(buf, start)?;
            if next > end {
                return Err(DnsWireError::BadResp);
            }
            Ok(Some(DnsRecord::Cname(name)))
        },
        TYPE_NS => {
            let (name, next) = read_name(buf, start)?;
            if next > end {
                return Err(DnsWireError::BadResp);
            }
            Ok(Some(DnsRecord::Ns(name)))
        },
        TYPE_PTR => {
            let (name, next) = read_name(buf, start)?;
            if next > end {
                return Err(DnsWireError::BadResp);
            }
            Ok(Some(DnsRecord::Ptr(name)))
        },
        TYPE_MX => {
            if rdlen < 3 {
                return Err(DnsWireError::BadResp);
            }
            let priority = u16::from_be_bytes([buf[start], buf[start + 1]]);
            let (exchange, next) = read_name(buf, start + 2)?;
            if next > end {
                return Err(DnsWireError::BadResp);
            }
            Ok(Some(DnsRecord::Mx { priority, exchange }))
        },
        TYPE_TXT => {
            let mut pos = start;
            let mut chunks = Vec::new();
            while pos < end {
                let (s, next) = read_character_string(buf, pos, end)?;
                chunks.push(s);
                pos = next;
            }
            Ok(Some(DnsRecord::Txt(chunks)))
        },
        TYPE_SRV => {
            if rdlen < 7 {
                return Err(DnsWireError::BadResp);
            }
            let priority = u16::from_be_bytes([buf[start], buf[start + 1]]);
            let weight = u16::from_be_bytes([buf[start + 2], buf[start + 3]]);
            let port = u16::from_be_bytes([buf[start + 4], buf[start + 5]]);
            let (name, next) = read_name(buf, start + 6)?;
            if next > end {
                return Err(DnsWireError::BadResp);
            }
            Ok(Some(DnsRecord::Srv {
                priority,
                weight,
                port,
                name,
            }))
        },
        TYPE_SOA => {
            let (nsname, p1) = read_name(buf, start)?;
            let (hostmaster, p2) = read_name(buf, p1)?;
            if p2 + 20 > end {
                return Err(DnsWireError::BadResp);
            }
            let serial = u32::from_be_bytes([buf[p2], buf[p2 + 1], buf[p2 + 2], buf[p2 + 3]]);
            let refresh = i32::from_be_bytes([buf[p2 + 4], buf[p2 + 5], buf[p2 + 6], buf[p2 + 7]]);
            let retry = i32::from_be_bytes([buf[p2 + 8], buf[p2 + 9], buf[p2 + 10], buf[p2 + 11]]);
            let expire =
                i32::from_be_bytes([buf[p2 + 12], buf[p2 + 13], buf[p2 + 14], buf[p2 + 15]]);
            let minttl =
                u32::from_be_bytes([buf[p2 + 16], buf[p2 + 17], buf[p2 + 18], buf[p2 + 19]]);
            Ok(Some(DnsRecord::Soa {
                nsname,
                hostmaster,
                serial,
                refresh,
                retry,
                expire,
                minttl,
            }))
        },
        TYPE_NAPTR => {
            if rdlen < 7 {
                return Err(DnsWireError::BadResp);
            }
            let order = u16::from_be_bytes([buf[start], buf[start + 1]]);
            let preference = u16::from_be_bytes([buf[start + 2], buf[start + 3]]);
            let (flags, p1) = read_character_string(buf, start + 4, end)?;
            let (service, p2) = read_character_string(buf, p1, end)?;
            let (regexp, p3) = read_character_string(buf, p2, end)?;
            let (replacement, next) = read_name(buf, p3)?;
            if next > end {
                return Err(DnsWireError::BadResp);
            }
            Ok(Some(DnsRecord::Naptr {
                order,
                preference,
                flags,
                service,
                regexp,
                replacement,
            }))
        },
        TYPE_CAA => {
            if rdlen < 2 {
                return Err(DnsWireError::BadResp);
            }
            let critical = buf[start];
            let tag_len = buf[start + 1] as usize;
            let tag_start = start + 2;
            let tag_end = tag_start
                .checked_add(tag_len)
                .ok_or(DnsWireError::BadResp)?;
            if tag_end > end {
                return Err(DnsWireError::BadResp);
            }
            let tag = String::from_utf8_lossy(&buf[tag_start..tag_end]).into_owned();
            let value = String::from_utf8_lossy(&buf[tag_end..end]).into_owned();
            Ok(Some(DnsRecord::Caa {
                critical,
                tag,
                value,
            }))
        },
        TYPE_TLSA => {
            if rdlen < 3 {
                return Err(DnsWireError::BadResp);
            }
            Ok(Some(DnsRecord::Tlsa {
                cert_usage: buf[start],
                selector: buf[start + 1],
                match_type: buf[start + 2],
                data: buf[start + 3..end].to_vec(),
            }))
        },
        _ => Ok(None),
    }
}

fn read_character_string(
    buf: &[u8],
    pos: usize,
    limit: usize,
) -> Result<(String, usize), DnsWireError> {
    if pos >= limit || pos >= buf.len() {
        return Err(DnsWireError::BadResp);
    }
    let len = buf[pos] as usize;
    let text_start = pos + 1;
    let text_end = text_start.checked_add(len).ok_or(DnsWireError::BadResp)?;
    if text_end > limit || text_end > buf.len() {
        return Err(DnsWireError::BadResp);
    }
    let s = String::from_utf8_lossy(&buf[text_start..text_end]).into_owned();
    Ok((s, text_end))
}

/// Decode a DNS domain name with RFC 1035 compression pointer support and cycle detection.
pub fn read_name(buf: &[u8], start: usize) -> Result<(String, usize), DnsWireError> {
    let mut labels: Vec<String> = Vec::new();
    let mut pos = start;
    let mut end_offset = None;
    let mut jumps = 0u8;

    loop {
        if pos >= buf.len() {
            return Err(DnsWireError::BadResp);
        }
        let len_byte = buf[pos];
        if len_byte == 0 {
            if end_offset.is_none() {
                end_offset = Some(pos + 1);
            }
            break;
        }
        match len_byte & 0xc0 {
            0xc0 => {
                if pos + 1 >= buf.len() {
                    return Err(DnsWireError::BadResp);
                }
                let ptr = (((len_byte & 0x3f) as usize) << 8) | (buf[pos + 1] as usize);
                if ptr >= buf.len() {
                    return Err(DnsWireError::BadResp);
                }
                if end_offset.is_none() {
                    end_offset = Some(pos + 2);
                }
                jumps += 1;
                if jumps > 64 {
                    return Err(DnsWireError::BadResp);
                }
                pos = ptr;
            },
            0x00 => {
                let len = len_byte as usize;
                let label_start = pos + 1;
                let label_end = label_start.checked_add(len).ok_or(DnsWireError::BadResp)?;
                if label_end > buf.len() {
                    return Err(DnsWireError::BadResp);
                }
                labels.push(String::from_utf8_lossy(&buf[label_start..label_end]).into_owned());
                pos = label_end;
            },
            _ => return Err(DnsWireError::BadResp),
        }
    }

    Ok((labels.join("."), end_offset.unwrap_or(start + 1)))
}

/// Convert an IP address into its `.in-addr.arpa` / `.ip6.arpa` reverse-lookup domain.
pub fn reverse_ip_to_arpa(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, c, d] = v4.octets();
            format!("{d}.{c}.{b}.{a}.in-addr.arpa")
        },
        IpAddr::V6(v6) => {
            let octets = v6.octets();
            let mut parts = Vec::with_capacity(34);
            for byte in octets.iter().rev() {
                parts.push(format!("{:x}", byte & 0x0f));
                parts.push(format!("{:x}", byte >> 4));
            }
            parts.push("ip6".to_string());
            parts.push("arpa".to_string());
            parts.join(".")
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_name_uncompressed(name: &str) -> Vec<u8> {
        let mut out = Vec::new();
        for label in name.split('.').filter(|s| !s.is_empty()) {
            out.push(label.len() as u8);
            out.extend_from_slice(label.as_bytes());
        }
        out.push(0);
        out
    }

    fn build_packet(id: u16, rcode: u8, tc: bool, answers: &[(u16, Vec<u8>)]) -> Vec<u8> {
        let mut pkt = encode_query(id, "example.com", TYPE_ANY).unwrap();
        pkt[2] = 0x81 | if tc { 0x02 } else { 0x00 };
        pkt[3] = 0x80 | (rcode & 0x0f);
        pkt[6..8].copy_from_slice(&(answers.len() as u16).to_be_bytes());
        for (rtype, rdata) in answers {
            pkt.extend_from_slice(&[0xc0, 0x0c]); // pointer to QNAME at offset 12
            pkt.extend_from_slice(&rtype.to_be_bytes());
            pkt.extend_from_slice(&1u16.to_be_bytes()); // IN
            pkt.extend_from_slice(&60u32.to_be_bytes());
            pkt.extend_from_slice(&(rdata.len() as u16).to_be_bytes());
            pkt.extend_from_slice(rdata);
        }
        pkt
    }

    #[test]
    fn encodes_and_validates_qname() {
        let q = encode_query(0x1234, "sub.example.com.", TYPE_A).unwrap();
        assert_eq!(&q[0..2], &[0x12, 0x34]);
        assert_eq!(&q[12..16], &[3, b's', b'u', b'b']);
        assert!(matches!(
            encode_query(1, "", TYPE_A),
            Err(DnsWireError::BadName)
        ));
        assert!(matches!(
            encode_query(1, "a..b.com", TYPE_A),
            Err(DnsWireError::BadName)
        ));
        let too_long = "a".repeat(64);
        assert!(matches!(
            encode_query(1, &too_long, TYPE_A),
            Err(DnsWireError::BadName)
        ));
    }

    #[test]
    fn decodes_all_record_types_and_compression() {
        let mut mx = Vec::new();
        mx.extend_from_slice(&10u16.to_be_bytes());
        mx.extend_from_slice(&encode_name_uncompressed("mail"));
        mx.pop(); // replace terminating 0 with pointer to example.com at offset 12 + 4?
        mx.clear();
        mx.extend_from_slice(&10u16.to_be_bytes());
        mx.extend_from_slice(&[4, b'm', b'a', b'i', b'l', 0xc0, 0x0c]);

        let mut srv = Vec::new();
        srv.extend_from_slice(&1u16.to_be_bytes());
        srv.extend_from_slice(&5u16.to_be_bytes());
        srv.extend_from_slice(&443u16.to_be_bytes());
        srv.extend_from_slice(&encode_name_uncompressed("svc.example.com"));

        let mut soa = Vec::new();
        soa.extend_from_slice(&encode_name_uncompressed("ns1.example.com"));
        soa.extend_from_slice(&encode_name_uncompressed("hostmaster.example.com"));
        soa.extend_from_slice(&2026u32.to_be_bytes());
        soa.extend_from_slice(&3600i32.to_be_bytes());
        soa.extend_from_slice(&600i32.to_be_bytes());
        soa.extend_from_slice(&86400i32.to_be_bytes());
        soa.extend_from_slice(&300u32.to_be_bytes());

        let mut naptr = Vec::new();
        naptr.extend_from_slice(&100u16.to_be_bytes());
        naptr.extend_from_slice(&10u16.to_be_bytes());
        naptr.push(1);
        naptr.push(b'u');
        naptr.push(7);
        naptr.extend_from_slice(b"E2U+sip");
        naptr.push(0);
        naptr.extend_from_slice(&encode_name_uncompressed("sip.example.com"));

        let mut caa = vec![128u8, 5];
        caa.extend_from_slice(b"issue");
        caa.extend_from_slice(b"ca.org");

        let tlsa = vec![3u8, 1, 1, 0xaa, 0xbb, 0xcc];

        let mut txt = vec![5u8];
        txt.extend_from_slice(b"hello");
        txt.push(5);
        txt.extend_from_slice(b"world");

        let pkt = build_packet(
            42,
            0,
            false,
            &[
                (TYPE_A, vec![1, 2, 3, 4]),
                (TYPE_AAAA, Ipv6Addr::LOCALHOST.octets().to_vec()),
                (TYPE_CNAME, vec![0xc0, 0x0c]),
                (TYPE_NS, encode_name_uncompressed("ns1.example.com")),
                (TYPE_PTR, encode_name_uncompressed("ptr.example.com")),
                (TYPE_MX, mx),
                (TYPE_TXT, txt),
                (TYPE_SRV, srv),
                (TYPE_SOA, soa),
                (TYPE_NAPTR, naptr),
                (TYPE_CAA, caa),
                (TYPE_TLSA, tlsa),
            ],
        );

        let decoded = decode_response(&pkt, 42).unwrap();
        assert!(!decoded.truncated);
        assert_eq!(decoded.answers.len(), 12);
        assert_eq!(
            decoded.answers[5],
            DnsRecord::Mx {
                priority: 10,
                exchange: "mail.example.com".to_string()
            }
        );
        assert_eq!(
            decoded.answers[6],
            DnsRecord::Txt(vec!["hello".to_string(), "world".to_string()])
        );
        assert_eq!(
            decoded.answers[10],
            DnsRecord::Caa {
                critical: 128,
                tag: "issue".to_string(),
                value: "ca.org".to_string()
            }
        );
        assert_eq!(
            decoded.answers[11],
            DnsRecord::Tlsa {
                cert_usage: 3,
                selector: 1,
                match_type: 1,
                data: vec![0xaa, 0xbb, 0xcc]
            }
        );
    }

    #[test]
    fn rejects_compression_loops_and_flags_truncation() {
        let mut loop_pkt = encode_query(7, "a.test", TYPE_A).unwrap();
        loop_pkt[2] = 0x81;
        loop_pkt[3] = 0x80;
        loop_pkt[6..8].copy_from_slice(&1u16.to_be_bytes());
        // Answer pointing directly back to itself
        let self_ptr_off = loop_pkt.len() as u16;
        let ptr_hi = 0xc0 | ((self_ptr_off >> 8) as u8);
        let ptr_lo = (self_ptr_off & 0xff) as u8;
        loop_pkt.extend_from_slice(&[ptr_hi, ptr_lo, 0, 1, 0, 1, 0, 0, 0, 1, 0, 4, 1, 2, 3, 4]);
        assert!(matches!(
            decode_response(&loop_pkt, 7),
            Err(DnsWireError::BadResp)
        ));

        let trunc = build_packet(8, 0, true, &[]);
        let decoded = decode_response(&trunc, 8).unwrap();
        assert!(decoded.truncated);
    }

    #[test]
    fn formats_reverse_arpa() {
        assert_eq!(
            reverse_ip_to_arpa("192.0.2.1".parse().unwrap()),
            "1.2.0.192.in-addr.arpa"
        );
        assert_eq!(
            reverse_ip_to_arpa("::1".parse().unwrap()),
            "1.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.ip6.arpa"
        );
    }
}
