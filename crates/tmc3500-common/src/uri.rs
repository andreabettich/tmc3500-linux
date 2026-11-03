//! `tmc3500://host[:port][/][?snmp-port=N&community=C]` device URIs. CUPS (lpadmin) only accepts a
//! query after a path, so write `tmc3500://host:9100/?snmp-port=1161`.
use std::fmt;

pub const SCHEME: &str = "tmc3500";
pub const DEFAULT_PORT: u16 = 9100;
const MAX_URI: usize = 1024;
const MAX_HOST: usize = 253;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceUri {
    /// Host name or IP address (IPv6 without brackets).
    pub host: String,
    pub port: u16,
    pub snmp_port: Option<u16>,
    pub community: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UriError(pub String);

impl fmt::Display for UriError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid device URI: {}", self.0)
    }
}

impl std::error::Error for UriError {}

fn err<T>(s: &str) -> Result<T, UriError> {
    Err(UriError(s.to_string()))
}

fn parse_port(s: &str) -> Result<u16, UriError> {
    if s.is_empty() || s.len() > 5 || !s.bytes().all(|c| c.is_ascii_digit()) {
        return err("bad port");
    }
    match s.parse::<u16>() {
        Ok(p) if p > 0 => Ok(p),
        _ => err("port out of range"),
    }
}

fn percent_decode(s: &str) -> Result<String, UriError> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' => {
                let hex = b.get(i + 1..i + 3).ok_or(UriError("bad %-escape".into()))?;
                let hex = std::str::from_utf8(hex).map_err(|_| UriError("bad %-escape".into()))?;
                let v = u8::from_str_radix(hex, 16).map_err(|_| UriError("bad %-escape".into()))?;
                out.push(v);
                i += 3;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    let s = String::from_utf8(out).map_err(|_| UriError("not UTF-8".into()))?;
    if s.chars().any(char::is_control) {
        return err("control characters");
    }
    Ok(s)
}

fn valid_hostname(h: &str) -> bool {
    !h.is_empty()
        && h.len() <= MAX_HOST
        && !h.starts_with('-')
        && !h.starts_with('.')
        && h.bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'.' || c == b'_')
}

impl DeviceUri {
    pub fn parse(s: &str) -> Result<DeviceUri, UriError> {
        if s.len() > MAX_URI {
            return err("too long");
        }
        let rest = s
            .strip_prefix(SCHEME)
            .and_then(|r| r.strip_prefix("://"))
            .ok_or(UriError(format!("scheme must be {SCHEME}://")))?;
        let (authority, query) = match rest.split_once('?') {
            Some((a, q)) => (a, Some(q)),
            None => (rest, None),
        };
        let authority = authority.strip_suffix('/').unwrap_or(authority);
        if authority.contains('/') {
            return err("paths are not supported");
        }
        if authority.contains('@') {
            return err("user info is not supported");
        }
        let (host, port) = if let Some(r) = authority.strip_prefix('[') {
            let (h, after) = r.split_once(']').ok_or(UriError("unclosed [".into()))?;
            if h.parse::<std::net::Ipv6Addr>().is_err() {
                return err("bad IPv6 address");
            }
            let port = match after {
                "" => DEFAULT_PORT,
                p => parse_port(p.strip_prefix(':').ok_or(UriError("junk after ]".into()))?)?,
            };
            (h.to_string(), port)
        } else {
            let (h, port) = match authority.split_once(':') {
                Some((h, p)) => (h, parse_port(p)?),
                None => (authority, DEFAULT_PORT),
            };
            if !valid_hostname(h) {
                return err("bad host name");
            }
            (h.to_string(), port)
        };
        let mut u = DeviceUri {
            host,
            port,
            snmp_port: None,
            community: None,
        };
        if let Some(q) = query {
            for pair in q.split('&').filter(|p| !p.is_empty()) {
                let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
                match k {
                    "snmp-port" => u.snmp_port = Some(parse_port(v)?),
                    "community" => {
                        let c = percent_decode(v)?;
                        if c.len() > 255 {
                            return err("community too long");
                        }
                        u.community = Some(c);
                    }
                    _ => return Err(UriError(format!("unknown option {k:?}"))),
                }
            }
        }
        Ok(u)
    }

    fn host_for_authority(&self) -> String {
        if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        }
    }

    /// `host:port` for the job connection (IPv6 in brackets).
    pub fn authority(&self) -> String {
        format!("{}:{}", self.host_for_authority(), self.port)
    }

    /// `host:snmp-port`, using the URI's snmp-port or `default_port`.
    pub fn snmp_authority(&self, default_port: u16) -> String {
        format!(
            "{}:{}",
            self.host_for_authority(),
            self.snmp_port.unwrap_or(default_port)
        )
    }

    /// The URI without the community (safe to log).
    pub fn redacted(&self) -> String {
        format!("{SCHEME}://{}", self.authority())
    }
}
