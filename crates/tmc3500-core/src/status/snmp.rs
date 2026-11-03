//! Minimal SNMP v1/v2c codec (BER subset): GetRequest, GetNextRequest, GetResponse.
//!
//! Strict on input: messages over [`MAX_MESSAGE`] bytes, nesting deeper than [`MAX_DEPTH`],
//! indefinite or inconsistent lengths, multi-byte tags, trailing bytes and out-of-range integers are
//! rejected with an [`SnmpError`]. Never panics.
use std::fmt;

/// Largest message accepted or produced.
pub const MAX_MESSAGE: usize = 64 * 1024;
/// Deepest TLV nesting accepted (message = 1, PDU = 2, varbind list = 3, varbind = 4).
pub const MAX_DEPTH: usize = 8;
/// Most variable bindings in one message.
pub const MAX_VARBINDS: usize = 256;
/// Most sub-identifiers in one OID.
pub const MAX_OID_LEN: usize = 128;
/// Longest community string.
pub const MAX_COMMUNITY: usize = 255;

const TAG_INTEGER: u8 = 0x02;
const TAG_OCTET_STRING: u8 = 0x04;
const TAG_NULL: u8 = 0x05;
const TAG_OID: u8 = 0x06;
const TAG_SEQUENCE: u8 = 0x30;
const TAG_IPADDRESS: u8 = 0x40;
const TAG_COUNTER32: u8 = 0x41;
const TAG_GAUGE32: u8 = 0x42;
const TAG_TIMETICKS: u8 = 0x43;
const TAG_COUNTER64: u8 = 0x46;
const TAG_NO_SUCH_OBJECT: u8 = 0x80;
const TAG_NO_SUCH_INSTANCE: u8 = 0x81;
const TAG_END_OF_MIB_VIEW: u8 = 0x82;
const TAG_GET: u8 = 0xa0;
const TAG_GET_NEXT: u8 = 0xa1;
const TAG_RESPONSE: u8 = 0xa2;

/// Object identifier (at least two arcs, first arc 0..=2).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Oid(Vec<u32>);

impl Oid {
    /// Parse dotted notation, e.g. `1.3.6.1.2.1.1.1.0`.
    pub fn parse(s: &str) -> Result<Oid, SnmpError> {
        let s = s.strip_prefix('.').unwrap_or(s);
        let mut v = Vec::new();
        for part in s.split('.') {
            if part.is_empty() || !part.bytes().all(|c| c.is_ascii_digit()) {
                return Err(SnmpError::BadOid);
            }
            v.push(part.parse::<u32>().map_err(|_| SnmpError::BadOid)?);
        }
        Oid::from_vec(v)
    }

    /// Validate and wrap sub-identifiers.
    pub fn from_vec(v: Vec<u32>) -> Result<Oid, SnmpError> {
        if v.len() < 2 || v.len() > MAX_OID_LEN || v[0] > 2 || (v[0] < 2 && v[1] >= 40) {
            return Err(SnmpError::BadOid);
        }
        // first two arcs are packed as 40*a+b, which must fit a u32
        if v[0] == 2 && v[1] > u32::MAX - 80 {
            return Err(SnmpError::BadOid);
        }
        Ok(Oid(v))
    }

    /// `0.0`, the null OID.
    pub fn zero_dot_zero() -> Oid {
        Oid(vec![0, 0])
    }

    pub fn as_slice(&self) -> &[u32] {
        &self.0
    }

    /// This OID with one more sub-identifier. Saturates at [`MAX_OID_LEN`] (returns `self`).
    pub fn child(&self, n: u32) -> Oid {
        let mut v = self.0.clone();
        if v.len() < MAX_OID_LEN {
            v.push(n);
        }
        Oid(v)
    }

    pub fn starts_with(&self, prefix: &Oid) -> bool {
        self.0.starts_with(&prefix.0)
    }
}

impl fmt::Display for Oid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, n) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str(".")?;
            }
            write!(f, "{n}")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Version {
    V1,
    V2c,
}

impl Version {
    fn wire(self) -> i64 {
        match self {
            Version::V1 => 0,
            Version::V2c => 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PduType {
    GetRequest,
    GetNextRequest,
    GetResponse,
}

impl PduType {
    fn tag(self) -> u8 {
        match self {
            PduType::GetRequest => TAG_GET,
            PduType::GetNextRequest => TAG_GET_NEXT,
            PduType::GetResponse => TAG_RESPONSE,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Integer(i64),
    OctetString(Vec<u8>),
    Null,
    ObjectId(Oid),
    IpAddress([u8; 4]),
    Counter32(u32),
    Gauge32(u32),
    TimeTicks(u32),
    Counter64(u64),
    NoSuchObject,
    NoSuchInstance,
    EndOfMibView,
    /// A well-formed value of a type this codec does not interpret (e.g. Opaque).
    Other {
        tag: u8,
        raw: Vec<u8>,
    },
}

impl Value {
    /// Integer-like values as i64.
    pub fn as_i64(&self) -> Option<i64> {
        match *self {
            Value::Integer(i) => Some(i),
            Value::Counter32(u) | Value::Gauge32(u) | Value::TimeTicks(u) => Some(i64::from(u)),
            Value::Counter64(u) => i64::try_from(u).ok(),
            _ => None,
        }
    }

    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Value::OctetString(b) => Some(b),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VarBind {
    pub oid: Oid,
    pub value: Value,
}

impl VarBind {
    pub fn new(oid: Oid, value: Value) -> Self {
        Self { oid, value }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub version: Version,
    pub community: Vec<u8>,
    pub pdu_type: PduType,
    pub request_id: i32,
    pub error_status: i64,
    pub error_index: i64,
    pub varbinds: Vec<VarBind>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnmpError {
    TooLarge,
    TooDeep,
    Truncated,
    BadLength,
    BadTag { expected: u8, found: u8 },
    BadInteger,
    BadOid,
    TrailingBytes,
    BadVersion(i64),
    UnexpectedPdu(u8),
    RequestIdMismatch { expected: i32, found: i32 },
    TooManyVarBinds,
    BadCommunity,
}

impl fmt::Display for SnmpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge => write!(f, "SNMP message larger than {MAX_MESSAGE} bytes"),
            Self::TooDeep => write!(f, "SNMP message nested deeper than {MAX_DEPTH}"),
            Self::Truncated => f.write_str("SNMP message truncated"),
            Self::BadLength => f.write_str("invalid BER length"),
            Self::BadTag { expected, found } => {
                write!(f, "expected BER tag {expected:#04x}, found {found:#04x}")
            }
            Self::BadInteger => f.write_str("invalid or out-of-range integer"),
            Self::BadOid => f.write_str("invalid object identifier"),
            Self::TrailingBytes => f.write_str("trailing bytes after SNMP message"),
            Self::BadVersion(v) => write!(f, "unsupported SNMP version {v}"),
            Self::UnexpectedPdu(t) => write!(f, "unexpected PDU type {t:#04x}"),
            Self::RequestIdMismatch { expected, found } => {
                write!(f, "request-id mismatch (expected {expected}, got {found})")
            }
            Self::TooManyVarBinds => write!(f, "more than {MAX_VARBINDS} variable bindings"),
            Self::BadCommunity => write!(f, "community longer than {MAX_COMMUNITY} bytes"),
        }
    }
}

impl std::error::Error for SnmpError {}

// ---------------------------------------------------------------- encoding

fn push_len(out: &mut Vec<u8>, n: usize) {
    if n < 0x80 {
        out.push(n as u8);
    } else {
        let bytes = (n as u64).to_be_bytes();
        let skip = bytes.iter().take_while(|&&b| b == 0).count();
        out.push(0x80 | (8 - skip) as u8);
        out.extend_from_slice(&bytes[skip..]);
    }
}

fn tlv(out: &mut Vec<u8>, tag: u8, content: &[u8]) {
    out.push(tag);
    push_len(out, content.len());
    out.extend_from_slice(content);
}

fn int_content(v: i64) -> Vec<u8> {
    let b = v.to_be_bytes();
    let mut start = 0;
    while start < 7 {
        let redundant = (b[start] == 0x00 && b[start + 1] & 0x80 == 0)
            || (b[start] == 0xff && b[start + 1] & 0x80 != 0);
        if !redundant {
            break;
        }
        start += 1;
    }
    b[start..].to_vec()
}

fn uint_content(v: u64) -> Vec<u8> {
    let mut out = vec![0];
    out.extend_from_slice(&v.to_be_bytes());
    let mut start = 0;
    while start < 8 && out[start] == 0 && out[start + 1] & 0x80 == 0 {
        start += 1;
    }
    out[start..].to_vec()
}

fn push_base128(out: &mut Vec<u8>, mut v: u32) {
    let mut tmp = [0u8; 5];
    let mut i = tmp.len();
    loop {
        i -= 1;
        tmp[i] = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            break;
        }
    }
    let last = tmp.len() - 1;
    for (j, b) in tmp.iter().enumerate().skip(i) {
        out.push(if j == last { *b } else { b | 0x80 });
    }
}

fn oid_content(o: &Oid) -> Vec<u8> {
    let v = &o.0;
    let mut out = Vec::with_capacity(v.len() + 4);
    // Oid invariants guarantee this cannot overflow.
    push_base128(&mut out, v[0] * 40 + v[1]);
    for &n in &v[2..] {
        push_base128(&mut out, n);
    }
    out
}

fn encode_value(out: &mut Vec<u8>, v: &Value) {
    match v {
        Value::Integer(i) => tlv(out, TAG_INTEGER, &int_content(*i)),
        Value::OctetString(b) => tlv(out, TAG_OCTET_STRING, b),
        Value::Null => tlv(out, TAG_NULL, &[]),
        Value::ObjectId(o) => tlv(out, TAG_OID, &oid_content(o)),
        Value::IpAddress(a) => tlv(out, TAG_IPADDRESS, a),
        Value::Counter32(u) => tlv(out, TAG_COUNTER32, &uint_content(u64::from(*u))),
        Value::Gauge32(u) => tlv(out, TAG_GAUGE32, &uint_content(u64::from(*u))),
        Value::TimeTicks(u) => tlv(out, TAG_TIMETICKS, &uint_content(u64::from(*u))),
        Value::Counter64(u) => tlv(out, TAG_COUNTER64, &uint_content(*u)),
        Value::NoSuchObject => tlv(out, TAG_NO_SUCH_OBJECT, &[]),
        Value::NoSuchInstance => tlv(out, TAG_NO_SUCH_INSTANCE, &[]),
        Value::EndOfMibView => tlv(out, TAG_END_OF_MIB_VIEW, &[]),
        Value::Other { tag, raw } => tlv(out, *tag, raw),
    }
}

/// Encode a message. Fails if it would exceed [`MAX_MESSAGE`] or other limits.
pub fn encode(m: &Message) -> Result<Vec<u8>, SnmpError> {
    if m.varbinds.len() > MAX_VARBINDS {
        return Err(SnmpError::TooManyVarBinds);
    }
    if m.community.len() > MAX_COMMUNITY {
        return Err(SnmpError::BadCommunity);
    }
    let mut list = Vec::new();
    for vb in &m.varbinds {
        let mut one = Vec::new();
        tlv(&mut one, TAG_OID, &oid_content(&vb.oid));
        encode_value(&mut one, &vb.value);
        tlv(&mut list, TAG_SEQUENCE, &one);
        if list.len() > MAX_MESSAGE {
            return Err(SnmpError::TooLarge);
        }
    }
    let mut pdu = Vec::new();
    tlv(&mut pdu, TAG_INTEGER, &int_content(i64::from(m.request_id)));
    tlv(&mut pdu, TAG_INTEGER, &int_content(m.error_status));
    tlv(&mut pdu, TAG_INTEGER, &int_content(m.error_index));
    tlv(&mut pdu, TAG_SEQUENCE, &list);
    let mut msg = Vec::new();
    tlv(&mut msg, TAG_INTEGER, &int_content(m.version.wire()));
    tlv(&mut msg, TAG_OCTET_STRING, &m.community);
    tlv(&mut msg, m.pdu_type.tag(), &pdu);
    let mut out = Vec::with_capacity(msg.len() + 6);
    tlv(&mut out, TAG_SEQUENCE, &msg);
    if out.len() > MAX_MESSAGE {
        return Err(SnmpError::TooLarge);
    }
    Ok(out)
}

/// Encode a Get/GetNext request for `oids` (values are NULL).
pub fn encode_request(
    version: Version,
    community: &[u8],
    pdu_type: PduType,
    request_id: i32,
    oids: &[Oid],
) -> Result<Vec<u8>, SnmpError> {
    encode(&Message {
        version,
        community: community.to_vec(),
        pdu_type,
        request_id,
        error_status: 0,
        error_index: 0,
        varbinds: oids
            .iter()
            .map(|o| VarBind::new(o.clone(), Value::Null))
            .collect(),
    })
}

// ---------------------------------------------------------------- decoding

struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
    depth: usize,
}

impl<'a> Reader<'a> {
    fn new(b: &'a [u8], depth: usize) -> Result<Self, SnmpError> {
        if depth > MAX_DEPTH {
            return Err(SnmpError::TooDeep);
        }
        Ok(Reader { b, pos: 0, depth })
    }

    fn at_end(&self) -> bool {
        self.pos >= self.b.len()
    }

    fn byte(&mut self) -> Result<u8, SnmpError> {
        let c = *self.b.get(self.pos).ok_or(SnmpError::Truncated)?;
        self.pos += 1;
        Ok(c)
    }

    /// Next TLV: (tag, content).
    fn tlv(&mut self) -> Result<(u8, &'a [u8]), SnmpError> {
        let tag = self.byte()?;
        if tag & 0x1f == 0x1f {
            return Err(SnmpError::BadTag {
                expected: 0,
                found: tag,
            });
        }
        let first = self.byte()?;
        let len = if first < 0x80 {
            usize::from(first)
        } else {
            let n = usize::from(first & 0x7f);
            if n == 0 || n > 4 {
                return Err(SnmpError::BadLength);
            }
            let mut len = 0usize;
            for _ in 0..n {
                len = (len << 8) | usize::from(self.byte()?);
            }
            if len > MAX_MESSAGE {
                return Err(SnmpError::TooLarge);
            }
            len
        };
        let end = self.pos.checked_add(len).ok_or(SnmpError::BadLength)?;
        let content = self.b.get(self.pos..end).ok_or(SnmpError::Truncated)?;
        self.pos = end;
        Ok((tag, content))
    }

    fn expect(&mut self, expected: u8) -> Result<&'a [u8], SnmpError> {
        let (tag, c) = self.tlv()?;
        if tag != expected {
            return Err(SnmpError::BadTag {
                expected,
                found: tag,
            });
        }
        Ok(c)
    }

    fn child(&self, content: &'a [u8]) -> Result<Reader<'a>, SnmpError> {
        Reader::new(content, self.depth + 1)
    }

    fn finish(&self) -> Result<(), SnmpError> {
        if self.at_end() {
            Ok(())
        } else {
            Err(SnmpError::TrailingBytes)
        }
    }
}

fn parse_int(c: &[u8]) -> Result<i64, SnmpError> {
    if c.is_empty() || c.len() > 9 {
        return Err(SnmpError::BadInteger);
    }
    let negative = c[0] & 0x80 != 0;
    let mut v: i128 = if negative { -1 } else { 0 };
    for &b in c {
        v = (v << 8) | i128::from(b);
    }
    i64::try_from(v).map_err(|_| SnmpError::BadInteger)
}

fn parse_uint(c: &[u8], max: u64) -> Result<u64, SnmpError> {
    if c.is_empty() || c.len() > 9 || c[0] & 0x80 != 0 {
        return Err(SnmpError::BadInteger);
    }
    let mut v: u128 = 0;
    for &b in c {
        v = (v << 8) | u128::from(b);
    }
    match u64::try_from(v) {
        Ok(v) if v <= max => Ok(v),
        _ => Err(SnmpError::BadInteger),
    }
}

fn parse_oid(c: &[u8]) -> Result<Oid, SnmpError> {
    if c.is_empty() {
        return Err(SnmpError::BadOid);
    }
    let mut subs: Vec<u32> = Vec::new();
    let mut cur: u32 = 0;
    let mut in_sub = false;
    for &b in c {
        if !in_sub && b == 0x80 {
            return Err(SnmpError::BadOid); // non-minimal sub-identifier
        }
        if cur > (u32::MAX >> 7) {
            return Err(SnmpError::BadOid);
        }
        cur = (cur << 7) | u32::from(b & 0x7f);
        if b & 0x80 != 0 {
            in_sub = true;
        } else {
            if subs.is_empty() {
                let (a, rest) = match cur {
                    0..=39 => (0, cur),
                    40..=79 => (1, cur - 40),
                    _ => (2, cur - 80),
                };
                subs.push(a);
                subs.push(rest);
            } else {
                subs.push(cur);
            }
            if subs.len() > MAX_OID_LEN {
                return Err(SnmpError::BadOid);
            }
            cur = 0;
            in_sub = false;
        }
    }
    if in_sub {
        return Err(SnmpError::BadOid);
    }
    Oid::from_vec(subs)
}

/// Check that a constructed value nests no deeper than allowed.
fn check_nesting(content: &[u8], depth: usize) -> Result<(), SnmpError> {
    let mut r = Reader::new(content, depth)?;
    while !r.at_end() {
        let (tag, c) = r.tlv()?;
        if tag & 0x20 != 0 {
            check_nesting(c, depth + 1)?;
        }
    }
    Ok(())
}

fn exact_empty(c: &[u8], v: Value) -> Result<Value, SnmpError> {
    if c.is_empty() {
        Ok(v)
    } else {
        Err(SnmpError::BadLength)
    }
}

fn parse_value(tag: u8, c: &[u8], depth: usize) -> Result<Value, SnmpError> {
    Ok(match tag {
        TAG_INTEGER => Value::Integer(parse_int(c)?),
        TAG_OCTET_STRING => Value::OctetString(c.to_vec()),
        TAG_NULL => exact_empty(c, Value::Null)?,
        TAG_OID => Value::ObjectId(parse_oid(c)?),
        TAG_IPADDRESS => {
            Value::IpAddress(<[u8; 4]>::try_from(c).map_err(|_| SnmpError::BadLength)?)
        }
        TAG_COUNTER32 => Value::Counter32(parse_uint(c, u64::from(u32::MAX))? as u32),
        TAG_GAUGE32 => Value::Gauge32(parse_uint(c, u64::from(u32::MAX))? as u32),
        TAG_TIMETICKS => Value::TimeTicks(parse_uint(c, u64::from(u32::MAX))? as u32),
        TAG_COUNTER64 => Value::Counter64(parse_uint(c, u64::MAX)?),
        TAG_NO_SUCH_OBJECT => exact_empty(c, Value::NoSuchObject)?,
        TAG_NO_SUCH_INSTANCE => exact_empty(c, Value::NoSuchInstance)?,
        TAG_END_OF_MIB_VIEW => exact_empty(c, Value::EndOfMibView)?,
        _ => {
            if tag & 0x20 != 0 {
                check_nesting(c, depth + 1)?;
            }
            Value::Other {
                tag,
                raw: c.to_vec(),
            }
        }
    })
}

/// Decode any supported SNMP v1/v2c message (request or response).
pub fn decode(b: &[u8]) -> Result<Message, SnmpError> {
    if b.len() > MAX_MESSAGE {
        return Err(SnmpError::TooLarge);
    }
    let mut top = Reader::new(b, 0)?;
    let msg_c = top.expect(TAG_SEQUENCE)?;
    top.finish()?;
    let mut msg = top.child(msg_c)?;
    let version = match parse_int(msg.expect(TAG_INTEGER)?)? {
        0 => Version::V1,
        1 => Version::V2c,
        v => return Err(SnmpError::BadVersion(v)),
    };
    let community = msg.expect(TAG_OCTET_STRING)?;
    if community.len() > MAX_COMMUNITY {
        return Err(SnmpError::BadCommunity);
    }
    let (pdu_tag, pdu_c) = msg.tlv()?;
    msg.finish()?;
    let pdu_type = match pdu_tag {
        TAG_GET => PduType::GetRequest,
        TAG_GET_NEXT => PduType::GetNextRequest,
        TAG_RESPONSE => PduType::GetResponse,
        t => return Err(SnmpError::UnexpectedPdu(t)),
    };
    let mut pdu = msg.child(pdu_c)?;
    let request_id =
        i32::try_from(parse_int(pdu.expect(TAG_INTEGER)?)?).map_err(|_| SnmpError::BadInteger)?;
    let error_status = parse_int(pdu.expect(TAG_INTEGER)?)?;
    let error_index = parse_int(pdu.expect(TAG_INTEGER)?)?;
    let list_c = pdu.expect(TAG_SEQUENCE)?;
    pdu.finish()?;
    let mut list = pdu.child(list_c)?;
    let mut varbinds = Vec::new();
    while !list.at_end() {
        if varbinds.len() >= MAX_VARBINDS {
            return Err(SnmpError::TooManyVarBinds);
        }
        let vb_c = list.expect(TAG_SEQUENCE)?;
        let mut vb = list.child(vb_c)?;
        let oid = parse_oid(vb.expect(TAG_OID)?)?;
        let (tag, c) = vb.tlv()?;
        let value = parse_value(tag, c, vb.depth)?;
        vb.finish()?;
        varbinds.push(VarBind { oid, value });
    }
    Ok(Message {
        version,
        community: community.to_vec(),
        pdu_type,
        request_id,
        error_status,
        error_index,
        varbinds,
    })
}

/// Decode a GetResponse and check it answers request `expected_id`.
pub fn decode_response(b: &[u8], expected_id: i32) -> Result<Message, SnmpError> {
    let m = decode(b)?;
    if m.pdu_type != PduType::GetResponse {
        return Err(SnmpError::UnexpectedPdu(m.pdu_type.tag()));
    }
    if m.request_id != expected_id {
        return Err(SnmpError::RequestIdMismatch {
            expected: expected_id,
            found: m.request_id,
        });
    }
    Ok(m)
}

#[cfg(test)]
mod tests;
