//! proxy implementation, split from the parent subsystem.

use super::*;

/// An explicitly enabled, loopback-only ingress for our independent TLS proxy.
/// SO_REUSEPORT lets a copyover successor bind before its predecessor exits.
/// The ordinary Telnet listener never interprets PROXY headers.
pub(super) fn bind_proxy_listener() -> Option<TcpListener> {
    let configured = std::env::var("GRIM_PROXY_PORT")
        .ok()
        .or_else(|| std::fs::read_to_string("data/tls-proxy-port.txt").ok())?;
    let port = configured.trim().parse::<u16>().ok().filter(|p| *p != 0);
    let Some(port) = port else {
        error!("invalid GRIM_PROXY_PORT; trusted proxy ingress disabled");
        return None;
    };
    let bind = || -> std::io::Result<TcpListener> {
        let socket = TcpSocket::new_v4()?;
        socket.set_reuseaddr(true)?;
        socket.set_reuseport(true)?;
        socket.bind(([127, 0, 0, 1], port).into())?;
        socket.listen(128)
    };
    match bind() {
        Ok(listener) => {
            info!("Trusted TLS proxy ingress listening on 127.0.0.1:{port}");
            Some(listener)
        }
        Err(e) => {
            error!("failed to bind configured trusted proxy ingress: {e}");
            std::process::exit(1);
        }
    }
}

pub(super) fn parse_proxy_header(header: &[u8]) -> Option<SocketAddr> {
    if header.len() > 108 || !header.ends_with(b"\r\n") {
        return None;
    }
    let line = std::str::from_utf8(&header[..header.len() - 2]).ok()?;
    let fields: Vec<_> = line.split(' ').collect();
    if fields.len() != 6 || fields[0] != "PROXY" {
        return None;
    }
    let source = fields[2].parse::<std::net::IpAddr>().ok()?;
    let destination = fields[3].parse::<std::net::IpAddr>().ok()?;
    match fields[1] {
        "TCP4" if source.is_ipv4() && destination.is_ipv4() => {}
        "TCP6" if source.is_ipv6() && destination.is_ipv6() => {}
        _ => return None,
    }
    let port = fields[4].parse::<u16>().ok().filter(|port| *port > 0)?;
    fields[5].parse::<u16>().ok().filter(|port| *port > 0)?;
    Some(SocketAddr::new(source, port))
}

pub(super) async fn read_proxy_header(socket: &mut TcpStream) -> Option<SocketAddr> {
    tokio::time::timeout(Duration::from_secs(2), async {
        let mut header = Vec::with_capacity(108);
        while header.len() < 108 {
            header.push(socket.read_u8().await.ok()?);
            if header.ends_with(b"\r\n") {
                return parse_proxy_header(&header);
            }
        }
        None
    })
    .await
    .ok()
    .flatten()
}
