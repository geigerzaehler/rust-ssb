//! Discover and announce SSB peers on the local network.

use futures::prelude::*;

/// The default port used for discovery by SSB
pub const PORT: u16 = 8008;

/// Continuously announce `multi_address` by broadcasting it over the local network.
///
/// Send the announcement via UDP to the broadcast address on every IPv4 enabled interface.
pub async fn announce(
    multi_address: &crate::multi_address::MultiAddress,
    port: u16,
    interval: std::time::Duration,
) -> anyhow::Result<()> {
    let multi_address = multi_address.to_string();
    let broadcast_address = std::net::SocketAddrV4::new(std::net::Ipv4Addr::BROADCAST, port);
    let sockets = interface_addresses_ipv4()?
        .map(|ipv4_addr| {
            let addr = std::net::SocketAddrV4::new(ipv4_addr, port);
            broadcast_socket(addr)
        })
        .collect::<Result<Vec<_>, _>>()?;
    loop {
        for socket in &sockets {
            socket
                .send_to(multi_address.as_ref(), &broadcast_address)
                .await?;
        }
        async_std::task::sleep(interval).await;
    }
}

/// Create a UPD socket that binds to the given address and can broadcast.
///
/// `SO_REUSEADDR` is set so that the bind address can be reused.
fn broadcast_socket(addr: std::net::SocketAddrV4) -> std::io::Result<async_std::net::UdpSocket> {
    let socket = socket2::Socket::new(socket2::Domain::ipv4(), socket2::Type::dgram(), None)?;
    socket.set_broadcast(true)?;
    socket.set_reuse_address(true)?;
    socket.bind(&socket2::SockAddr::from(addr))?;
    Ok(socket.into_udp_socket().into())
}

/// Get all IPv4 addresses of network interfaces.
fn interface_addresses_ipv4() -> anyhow::Result<impl Iterator<Item = std::net::Ipv4Addr>> {
    let addresses = nix::ifaddrs::getifaddrs()?.filter_map(move |interface| {
        if let Some(nix::sys::socket::SockAddr::Inet(addr)) = interface.address {
            match addr.to_std() {
                std::net::SocketAddr::V4(addr) => Some(*addr.ip()),
                std::net::SocketAddr::V6(_) => None,
            }
        } else {
            None
        }
    });
    Ok(addresses)
}

/// Listen for multi address broadcast announcements on the given port and return
/// a stream of announcements.
pub fn discover(
    port: u16,
) -> std::io::Result<impl Stream<Item = anyhow::Result<crate::multi_address::MultiAddress>>> {
    let socket = broadcast_listen_socket(port)?;
    let stream = upd_socket_stream(socket).map(|bytes_result| {
        let bytes = bytes_result?;
        let data = String::from_utf8(bytes)?;
        let multi_address = data.parse()?;
        Ok(multi_address)
    });
    Ok(stream)
}

/// Creates a IPv4 UDP socket that listens for broadcast messages on all interfaces
fn broadcast_listen_socket(port: u16) -> std::io::Result<async_std::net::UdpSocket> {
    let socket = socket2::Socket::new(socket2::Domain::ipv4(), socket2::Type::dgram(), None)?;
    let bind_addr = std::net::SocketAddrV4::new(std::net::Ipv4Addr::BROADCAST, port);
    socket.set_reuse_address(true)?;
    socket.bind(&bind_addr.into())?;
    Ok(socket.into_udp_socket().into())
}

fn upd_socket_stream(
    socket: async_std::net::UdpSocket,
) -> impl Stream<Item = std::io::Result<Vec<u8>>> {
    let socket = std::sync::Arc::new(socket);
    futures::stream::repeat(socket).flat_map(|socket| {
        async move {
            let mut buf = vec![0u8; 2048];
            match socket.recv(&mut buf).await {
                Ok(size) => {
                    buf.resize(size, 0);
                    Ok(buf)
                }
                Err(err) => Err(err),
            }
        }
        .into_stream()
    })
}
