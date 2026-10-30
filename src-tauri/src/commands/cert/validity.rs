use super::*;
use base64::Engine as _;

/// 极简 DER TLV 读取器（仅用于解析 X.509 证书有效期）。
pub(super) struct DerReader<'a> {
    pub(super) buf: &'a [u8],
    pub(super) pos: usize,
}

impl<'a> DerReader<'a> {
    pub(super) fn new(buf: &'a [u8]) -> Self {
        DerReader { buf, pos: 0 }
    }

    /// 读取一个 TLV，返回 (tag, value)。
    pub(super) fn read_tlv(&mut self) -> Option<(u8, &'a [u8])> {
        if self.pos >= self.buf.len() {
            return None;
        }
        let tag = self.buf[self.pos];
        self.pos += 1;
        if self.pos >= self.buf.len() {
            return None;
        }
        let first = self.buf[self.pos];
        self.pos += 1;
        let len = if first & 0x80 == 0 {
            first as usize
        } else {
            let n = (first & 0x7f) as usize;
            if n == 0 || n > 4 || self.pos + n > self.buf.len() {
                return None;
            }
            let mut l = 0usize;
            for _ in 0..n {
                l = (l << 8) | self.buf[self.pos] as usize;
                self.pos += 1;
            }
            l
        };
        if self.pos + len > self.buf.len() {
            return None;
        }
        let value = &self.buf[self.pos..self.pos + len];
        self.pos += len;
        Some((tag, value))
    }

    pub(super) fn read_children(&mut self) -> Vec<(u8, &'a [u8])> {
        let mut out = Vec::new();
        while let Some(tlv) = self.read_tlv() {
            out.push(tlv);
        }
        out
    }
}

/// 解析 DER Time（UTCTime 12 位 / GeneralizedTime 14 位，Z 结尾）。
pub(super) fn parse_der_time(v: &[u8]) -> Option<DateTime<Utc>> {
    let s = std::str::from_utf8(v).ok()?;
    let s = s.trim_end_matches('Z');
    let s = s.split('.').next().unwrap_or(s); // 去掉小数秒
    let d: Vec<u8> = s.bytes().filter(|b| b.is_ascii_digit()).collect();
    let (year, month, day, hour, min, sec) = match d.len() {
        12 => {
            let yy = (d[0] - b'0') as i32 * 10 + (d[1] - b'0') as i32;
            let year = if yy >= 50 { 1900 + yy } else { 2000 + yy };
            (
                year,
                (d[2] - b'0') as u32 * 10 + (d[3] - b'0') as u32,
                (d[4] - b'0') as u32 * 10 + (d[5] - b'0') as u32,
                (d[6] - b'0') as u32 * 10 + (d[7] - b'0') as u32,
                (d[8] - b'0') as u32 * 10 + (d[9] - b'0') as u32,
                (d[10] - b'0') as u32 * 10 + (d[11] - b'0') as u32,
            )
        }
        14 => {
            let year = (d[0] - b'0') as i32 * 1000
                + (d[1] - b'0') as i32 * 100
                + (d[2] - b'0') as i32 * 10
                + (d[3] - b'0') as i32;
            (
                year,
                (d[4] - b'0') as u32 * 10 + (d[5] - b'0') as u32,
                (d[6] - b'0') as u32 * 10 + (d[7] - b'0') as u32,
                (d[8] - b'0') as u32 * 10 + (d[9] - b'0') as u32,
                (d[10] - b'0') as u32 * 10 + (d[11] - b'0') as u32,
                (d[12] - b'0') as u32 * 10 + (d[13] - b'0') as u32,
            )
        }
        _ => return None,
    };
    Utc.with_ymd_and_hms(year, month, day, hour, min, sec)
        .single()
}

/// 从证书 PEM 解析 notBefore / notAfter（RFC3339）。
pub(super) fn extract_validity(crt: &str) -> (Option<String>, Option<String>) {
    // 提取 base64 主体
    let mut b64 = String::new();
    let mut in_pem = false;
    for line in crt.lines() {
        let l = line.trim();
        if l.starts_with("-----BEGIN") {
            in_pem = true;
            continue;
        }
        if l.starts_with("-----END") {
            break;
        }
        if in_pem {
            b64.push_str(l);
        }
    }
    let der = match B64.decode(b64.trim()) {
        Ok(d) => d,
        Err(_) => return (None, None),
    };
    // Certificate ::= SEQUENCE { tbsCertificate, signatureAlgorithm, signatureValue }
    let mut r = DerReader::new(&der);
    let cert_val = match r.read_tlv() {
        Some((0x30, v)) => v,
        _ => return (None, None),
    };
    // 外层子项的第一个才是 TBSCertificate（其后的 signatureAlgorithm /
    // signatureValue 不是 TBSCertificate 字段）。必须再下降一层，
    // 否则会拿 tbs/sigAlg/sigValue 当字段遍历，validity 永远找不到 →
    // 有效期恒为 None → 调度器因 not_after 缺失永不触发自动续期。
    let tbs = match DerReader::new(cert_val).read_tlv() {
        Some((0x30, v)) => v,
        _ => return (None, None),
    };
    // TBSCertificate ::= SEQUENCE { version?, serialNumber, signature, issuer, validity, ... }
    let mut tr = DerReader::new(tbs);
    let children = tr.read_children();
    let mut seq_idx = 0usize;
    let mut validity: Option<&[u8]> = None;
    for (tag, val) in &children {
        if seq_idx == 0 && *tag == 0xA0 {
            continue; // version [0] EXPLICIT
        }
        if seq_idx == 3 {
            validity = Some(val);
            break;
        }
        seq_idx += 1;
    }
    let validity = match validity {
        Some(v) => v,
        None => return (None, None),
    };
    parse_validity_value(validity)
}

/// 解析 Validity ::= SEQUENCE { notBefore Time, notAfter Time }。
pub(super) fn parse_validity_value(validity: &[u8]) -> (Option<String>, Option<String>) {
    let mut nb = None;
    let mut na = None;
    for (tag, val) in DerReader::new(validity).read_children() {
        if tag == 0x17 || tag == 0x18 {
            if nb.is_none() {
                nb = parse_der_time(val);
            } else {
                na = parse_der_time(val);
            }
        }
    }
    (nb.map(|d| d.to_rfc3339()), na.map(|d| d.to_rfc3339()))
}
