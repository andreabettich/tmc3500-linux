//! TCP send-queue inspection without `unsafe`: reads the `tx_queue` column of
//! `/proc/net/tcp` / `/proc/net/tcp6` for one connection.
use std::net::{SocketAddr, TcpStream};

fn hex_addr(a: &SocketAddr) -> String {
    // The kernel prints each 32-bit word of the (network order) address as a native u32.
    match a {
        SocketAddr::V4(v) => format!(
            "{:08X}:{:04X}",
            u32::from_ne_bytes(v.ip().octets()),
            v.port()
        ),
        SocketAddr::V6(v) => {
            let o = v.ip().octets();
            let mut s = String::new();
            for c in o.as_chunks::<4>().0 {
                s.push_str(&format!("{:08X}", u32::from_ne_bytes(*c)));
            }
            format!("{s}:{:04X}", v.port())
        }
    }
}

/// `tx_queue` of the connection `local` → `peer` in a `/proc/net/tcp*` table.
pub fn parse_tx_queue(table: &str, local: &SocketAddr, peer: &SocketAddr) -> Option<u64> {
    let (l, p) = (hex_addr(local), hex_addr(peer));
    for line in table.lines().skip(1) {
        let mut f = line.split_whitespace();
        let (_sl, la, ra, _st, q) = (f.next()?, f.next()?, f.next()?, f.next()?, f.next()?);
        if la.eq_ignore_ascii_case(&l) && ra.eq_ignore_ascii_case(&p) {
            let tx = q.split(':').next()?;
            return u64::from_str_radix(tx, 16).ok();
        }
    }
    None
}

/// Bytes written to `stream` that the peer has not acknowledged yet (`None` = cannot tell).
pub fn send_queue(stream: &TcpStream) -> Option<u64> {
    let local = stream.local_addr().ok()?;
    let peer = stream.peer_addr().ok()?;
    let file = if local.is_ipv4() {
        "/proc/net/tcp"
    } else {
        "/proc/net/tcp6"
    };
    use std::io::Read;
    let mut text = String::new();
    std::fs::File::open(file)
        .ok()?
        .take(64 * 1024 * 1024)
        .read_to_string(&mut text)
        .ok()?;
    parse_tx_queue(&text, &local, &peer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const TABLE: &str = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 0100007F:1F90 0100007F:D431 01 0001E240:00000000 00:00000000 00000000  1000        0 1 1 0 20 4 30 10 -1
";

    #[test]
    fn parses_the_matching_row() {
        if cfg!(target_endian = "little") {
            let l: SocketAddr = "127.0.0.1:8080".parse().unwrap();
            let p: SocketAddr = "127.0.0.1:54321".parse().unwrap();
            assert_eq!(parse_tx_queue(TABLE, &l, &p), Some(123_456));
            assert_eq!(parse_tx_queue(TABLE, &p, &l), None);
        }
        assert_eq!(
            parse_tx_queue("", &"[::1]:1".parse().unwrap(), &"[::1]:2".parse().unwrap()),
            None
        );
    }

    #[test]
    fn live_socket_queue_grows_when_the_peer_does_not_read() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let mut c = TcpStream::connect(l.local_addr().unwrap()).unwrap();
        let (_s, _) = l.accept().unwrap();
        assert_eq!(send_queue(&c), Some(0));
        c.set_write_timeout(Some(std::time::Duration::from_millis(200)))
            .unwrap();
        let buf = vec![0u8; 64 * 1024];
        for _ in 0..200 {
            if c.write(&buf).is_err() {
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(send_queue(&c).unwrap() > 0);
    }
}
