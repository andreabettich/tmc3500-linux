//! Blocking SNMP client over UDP with per-request timeout and retries.
use std::fmt;
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};
use tmc3500_core::status::printer::{counter_oids, status_oids, PrinterStatus};
use tmc3500_core::status::snmp::{self, Oid, PduType, SnmpError, VarBind, Version, MAX_MESSAGE};

/// SNMP error-status noSuchName (v1).
const NO_SUCH_NAME: i64 = 2;

#[derive(Debug)]
pub enum ClientError {
    /// Name resolution or socket setup failed.
    Io(String),
    /// No valid reply within timeout × (retries + 1).
    Timeout,
    /// The agent replied with something that is not a valid response.
    InvalidReply(SnmpError),
    /// The agent replied with an SNMP error-status.
    ErrorStatus { status: i64, index: i64 },
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "SNMP I/O error: {e}"),
            Self::Timeout => f.write_str("SNMP timeout (no reply)"),
            Self::InvalidReply(e) => write!(f, "invalid SNMP reply: {e}"),
            Self::ErrorStatus { status, index } => {
                write!(f, "SNMP error-status {status} (index {index})")
            }
        }
    }
}

impl std::error::Error for ClientError {}

#[derive(Debug, Clone)]
pub struct SnmpClient {
    pub addr: SocketAddr,
    pub community: String,
    pub version: Version,
    pub timeout: Duration,
    pub retries: u32,
}

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn next_request_id() -> i32 {
    let seed = std::process::id().wrapping_mul(2_654_435_761)
        ^ std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    (seed.wrapping_add(n.wrapping_mul(7919)) & 0x7fff_ffff) as i32
}

/// Resolve `host:port` (first address). Blocking DNS: use IP addresses for mission-critical setups.
pub fn resolve(authority: &str) -> Result<SocketAddr, ClientError> {
    authority
        .to_socket_addrs()
        .map_err(|e| ClientError::Io(format!("cannot resolve {authority}: {e}")))?
        .next()
        .ok_or_else(|| ClientError::Io(format!("no address for {authority}")))
}

impl SnmpClient {
    pub fn new(
        addr: SocketAddr,
        community: &str,
        version: Version,
        timeout: Duration,
        retries: u32,
    ) -> Self {
        SnmpClient {
            addr,
            community: community.to_string(),
            version,
            timeout,
            retries,
        }
    }

    fn socket(&self) -> Result<UdpSocket, ClientError> {
        let bind = if self.addr.is_ipv4() {
            "0.0.0.0:0"
        } else {
            "[::]:0"
        };
        let s = UdpSocket::bind(bind).map_err(|e| ClientError::Io(e.to_string()))?;
        s.connect(self.addr)
            .map_err(|e| ClientError::Io(e.to_string()))?;
        Ok(s)
    }

    /// One request/response exchange with retries.
    fn exchange(&self, pdu: PduType, oids: &[Oid]) -> Result<snmp::Message, ClientError> {
        let sock = self.socket()?;
        let mut buf = vec![0u8; MAX_MESSAGE];
        for _attempt in 0..=self.retries {
            let id = next_request_id();
            let req = snmp::encode_request(self.version, self.community.as_bytes(), pdu, id, oids)
                .map_err(ClientError::InvalidReply)?;
            if sock.send(&req).is_err() {
                // e.g. ICMP port unreachable reported on a connected socket: try again
                std::thread::sleep(self.timeout.min(Duration::from_millis(200)));
                continue;
            }
            let deadline = Instant::now() + self.timeout;
            loop {
                let now = Instant::now();
                if now >= deadline {
                    break;
                }
                if sock.set_read_timeout(Some(deadline - now)).is_err() {
                    break;
                }
                let n = match sock.recv(&mut buf) {
                    Ok(n) => n,
                    Err(e)
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) =>
                    {
                        break
                    }
                    Err(_) => {
                        // ICMP errors (connection refused) arrive here; wait out the attempt.
                        std::thread::sleep(
                            deadline
                                .saturating_duration_since(Instant::now())
                                .min(Duration::from_millis(50)),
                        );
                        continue;
                    }
                };
                match snmp::decode_response(&buf[..n], id) {
                    Ok(m) => return Ok(m),
                    // A late answer to an earlier request: keep waiting.
                    Err(SnmpError::RequestIdMismatch { .. }) => continue,
                    Err(e) => return Err(ClientError::InvalidReply(e)),
                }
            }
        }
        Err(ClientError::Timeout)
    }

    /// GET `oids`. With SNMPv1, OIDs the agent reports as noSuchName are dropped and the request
    /// is repeated (at most `oids.len()` times).
    pub fn get(&self, oids: &[Oid]) -> Result<Vec<VarBind>, ClientError> {
        let mut oids = oids.to_vec();
        loop {
            if oids.is_empty() {
                return Ok(Vec::new());
            }
            let m = self.exchange(PduType::GetRequest, &oids)?;
            match m.error_status {
                0 => return Ok(m.varbinds),
                NO_SUCH_NAME if m.error_index >= 1 && (m.error_index as usize) <= oids.len() => {
                    oids.remove(m.error_index as usize - 1);
                }
                status => {
                    return Err(ClientError::ErrorStatus {
                        status,
                        index: m.error_index,
                    })
                }
            }
        }
    }

    /// GET-NEXT for one OID.
    pub fn get_next(&self, oid: &Oid) -> Result<Option<VarBind>, ClientError> {
        let m = self.exchange(PduType::GetNextRequest, std::slice::from_ref(oid))?;
        if m.error_status == NO_SUCH_NAME {
            return Ok(None);
        }
        if m.error_status != 0 {
            return Err(ClientError::ErrorStatus {
                status: m.error_status,
                index: m.error_index,
            });
        }
        Ok(m.varbinds.into_iter().next())
    }

    /// Query the printer status (Host Resources + Printer MIB values).
    pub fn query_status(&self) -> Result<PrinterStatus, ClientError> {
        Ok(PrinterStatus::from_varbinds(&self.get(&status_oids())?))
    }

    /// Query status plus counters and sysDescr.
    pub fn query_full(&self) -> Result<PrinterStatus, ClientError> {
        let mut oids = status_oids();
        oids.extend(counter_oids());
        Ok(PrinterStatus::from_varbinds(&self.get(&oids)?))
    }
}
