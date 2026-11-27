//! The send API for your own devices: a second listener on the local network that serves only
//! the outbox, accepts only send-scope keys, and refuses anyone outside private networks
//! (home Wi-Fi, Tailscale and similar). Shared by the service, the CLI and the desktop app.
use rusqlite::{Connection, OptionalExtension};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};

pub const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS device_api(id INTEGER PRIMARY KEY CHECK(id=1), port INTEGER NOT NULL CHECK(port BETWEEN 1024 AND 65535))";
pub const DEFAULT_PORT: u16 = 47840;

/// The port when the device API is on.
pub fn port(db: &Connection) -> rusqlite::Result<Option<u16>> {
    db.query_row("SELECT port FROM device_api WHERE id=1", [], |r| r.get(0))
        .optional()
}
pub fn enable(db: &Connection, port: u16) -> rusqlite::Result<()> {
    db.execute_batch(SCHEMA)?;
    db.execute(
        "INSERT INTO device_api(id,port) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET port=excluded.port",
        [port],
    )
    .map(drop)
}
pub fn disable(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(SCHEMA)?;
    db.execute("DELETE FROM device_api", []).map(drop)
}

/// Loopback, private LAN ranges, link-local, Tailscale's 100.64.0.0/10 and IPv6 unique-local.
pub fn private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => private_v4(v4),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => private_v4(v4),
            None => {
                v6.is_loopback()
                    || (v6.segments()[0] & 0xfe00) == 0xfc00
                    || (v6.segments()[0] & 0xffc0) == 0xfe80
            }
        },
    }
}
fn private_v4(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || (a == 100 && (64..128).contains(&b))
}

/// The address other devices on this network use, found without sending anything.
pub fn lan_address() -> Option<IpAddr> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect((Ipv4Addr::new(192, 168, 0, 1), 9)).ok()?;
    Some(socket.local_addr().ok()?.ip()).filter(|ip| private(*ip) && !ip.is_loopback())
}
/// The URL to give your other devices for queueing texts.
pub fn url(port: u16, prefix: &str) -> String {
    let host = lan_address().unwrap_or(IpAddr::V6(Ipv6Addr::LOCALHOST));
    format!("http://{}{prefix}/outbox", SocketAddr::new(host, port))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_private_networks_and_a_saved_port() {
        for ok in [
            "127.0.0.1",
            "10.1.2.3",
            "172.20.0.5",
            "192.168.1.20",
            "100.101.1.2",
            "169.254.1.1",
            "::1",
            "fd7a:115c:a1e0::1",
            "fe80::1",
            "::ffff:192.168.1.2",
        ] {
            assert!(private(ok.parse().unwrap()), "{ok}");
        }
        for no in [
            "8.8.8.8",
            "100.128.0.1",
            "172.32.0.1",
            "2001:db8::1",
            "::ffff:1.1.1.1",
        ] {
            assert!(!private(no.parse().unwrap()), "{no}");
        }
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(SCHEMA).unwrap();
        assert_eq!(port(&db).unwrap(), None);
        enable(&db, 47900).unwrap();
        enable(&db, DEFAULT_PORT).unwrap();
        assert_eq!(port(&db).unwrap(), Some(DEFAULT_PORT));
        disable(&db).unwrap();
        assert_eq!(port(&db).unwrap(), None);
    }
}
