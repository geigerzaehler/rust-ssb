//! Discover and announce SSB peers on the local network.

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
