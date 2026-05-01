use crate::{
    ip_support::IpSupport,
    ipc::DiscoveredDevice,
    protocol::packets::{DEVICE_TIMEOUT, DISCOVERY_PORT, DiscoveryPacket},
};
use anyhow::{self as ah, Context as _, format_err as err};
use socket2::{Domain, Protocol, Socket, Type};
use std::{
    collections::{BTreeSet, HashMap},
    net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6},
    sync::Arc,
    time::Instant,
};
use tokio::{net::UdpSocket, sync::Mutex};
use uuid::Uuid;

pub type DeviceMap = Arc<Mutex<HashMap<Uuid, DiscoveredDevice>>>;

const IPV6_MULTICAST_ADDR: Ipv6Addr = Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0, 0, 1);

fn ipv6_broadcast_if_indices() -> Vec<u32> {
    let Ok(ifaces) = if_addrs::get_if_addrs() else {
        return vec![];
    };
    let mut indices = BTreeSet::new();
    for iface in ifaces {
        if iface.is_loopback() || iface.is_p2p() || !iface.is_oper_up() {
            continue;
        }
        if let if_addrs::IfAddr::V6(_) = iface.addr
            && let Some(idx) = iface.index
        {
            indices.insert(idx);
        }
    }
    indices.into_iter().collect()
}

fn ipv4_broadcast_targets() -> Vec<(Ipv4Addr, Ipv4Addr)> {
    let Ok(ifaces) = if_addrs::get_if_addrs() else {
        return vec![];
    };
    let mut targets = Vec::with_capacity(ifaces.len());
    for iface in ifaces {
        if iface.is_loopback() || iface.is_p2p() || !iface.is_oper_up() {
            continue;
        }
        if let if_addrs::IfAddr::V4(v4) = iface.addr {
            let broadcast = v4.broadcast.unwrap_or_else(|| {
                let ip = u32::from(v4.ip);
                let mask = u32::from(v4.netmask);
                Ipv4Addr::from(ip & mask | !mask)
            });
            targets.push((v4.ip, broadcast));
        }
    }
    targets
}

pub async fn create_ipv4_listener_socket() -> ah::Result<UdpSocket> {
    let socket =
        tokio::net::UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, DISCOVERY_PORT))
            .await?;
    socket.set_broadcast(true)?;
    Ok(socket)
}

pub async fn create_ipv6_listener_socket() -> ah::Result<UdpSocket> {
    let sock = Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP))?;
    sock.set_only_v6(true)?;
    sock.set_reuse_address(true)?;
    sock.set_nonblocking(true)?;
    sock.bind(&SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, DISCOVERY_PORT, 0, 0).into())?;
    let std_sock: std::net::UdpSocket = sock.into();
    let udp = UdpSocket::from_std(std_sock)?;
    for idx in ipv6_broadcast_if_indices() {
        if let Err(e) = udp.join_multicast_v6(&IPV6_MULTICAST_ADDR, idx) {
            log::debug!("Failed to join IPv6 multicast on interface {idx}: {e}");
        }
    }
    Ok(udp)
}

pub async fn broadcast_presence_ipv4(packet: &DiscoveryPacket) {
    let data = match rkyv::to_bytes::<rkyv::rancor::Error>(packet) {
        Ok(d) => d,
        Err(e) => {
            log::error!("Failed to serialize discovery packet: {e}");
            return;
        }
    };
    for (local_ip, broadcast_ip) in ipv4_broadcast_targets() {
        let sock = match Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP)) {
            Ok(s) => s,
            Err(e) => {
                log::debug!("Failed to create IPv4 socket for {local_ip}: {e}");
                continue;
            }
        };
        if let Err(e) = sock.set_broadcast(true) {
            log::debug!("Failed to set broadcast on socket for {local_ip}: {e}");
            continue;
        }
        if let Err(e) = sock.set_nonblocking(true) {
            log::debug!("Failed to set nonblocking on socket for {local_ip}: {e}");
            continue;
        }
        if let Err(e) = sock.bind(&SocketAddrV4::new(local_ip, 0).into()) {
            log::debug!("Failed to bind IPv4 socket to {local_ip}: {e}");
            continue;
        }
        let std_sock: std::net::UdpSocket = sock.into();
        let udp = match UdpSocket::from_std(std_sock) {
            Ok(u) => u,
            Err(e) => {
                log::debug!("Failed to convert IPv4 socket for {local_ip}: {e}");
                continue;
            }
        };
        let dest = SocketAddrV4::new(broadcast_ip, DISCOVERY_PORT);
        if let Err(e) = udp.send_to(&data, dest).await {
            log::debug!(
                "IPv4 broadcast send error on {local_ip} -> {broadcast_ip} (non-fatal): {e}"
            );
        }
    }
}

pub async fn broadcast_presence_ipv6(packet: &DiscoveryPacket) {
    let Ok(sock) = Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP)) else {
        log::debug!("Failed to create IPv6 socket for broadcasting.");
        return;
    };
    if let Err(e) = sock.set_only_v6(true) {
        log::debug!("Failed to set only_v6 on IPv6 socket for broadcasting: {e}");
        return;
    }
    if let Err(e) = sock.set_nonblocking(true) {
        log::debug!("Failed to set nonblocking on IPv6 socket for broadcasting: {e}");
        return;
    }
    if let Err(e) = sock.bind(&SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 0, 0, 0).into()) {
        log::debug!("Failed to bind IPv6 socket for broadcasting: {e}");
        return;
    }
    let std_sock: std::net::UdpSocket = sock.into();
    let socket = match UdpSocket::from_std(std_sock) {
        Ok(s) => s,
        Err(e) => {
            log::debug!("Failed to convert IPv6 socket for broadcasting: {e}");
            return;
        }
    };

    let data = match rkyv::to_bytes::<rkyv::rancor::Error>(packet) {
        Ok(d) => d,
        Err(e) => {
            log::error!("Failed to serialize discovery packet: {e}");
            return;
        }
    };
    for idx in ipv6_broadcast_if_indices() {
        let addr = SocketAddrV6::new(IPV6_MULTICAST_ADDR, DISCOVERY_PORT, 0, idx);
        if let Err(e) = socket.send_to(&data, addr).await {
            log::debug!("IPv6 multicast send error on interface {idx} (non-fatal): {e}");
        }
    }
}

async fn update_device(
    devices: &DeviceMap,
    packet: DiscoveryPacket,
    addr: SocketAddr,
) -> ah::Result<()> {
    let device_addr = match addr {
        SocketAddr::V6(v6) => {
            if !IpSupport::ipv6() {
                return Err(err!(
                    "Received discovery packet from IPv6 address {addr}, but IPv6 support is disabled"
                ));
            }
            SocketAddr::V6(SocketAddrV6::new(
                *v6.ip(),
                packet.transfer_port,
                v6.flowinfo(),
                v6.scope_id(),
            ))
        }
        SocketAddr::V4(_) => {
            if !IpSupport::ipv4() {
                return Err(err!(
                    "Received discovery packet from IPv4 address {addr}, but IPv4 support is disabled"
                ));
            }
            SocketAddr::new(addr.ip(), packet.transfer_port)
        }
    };

    {
        let mut map = devices.lock().await;

        let mut insert = false;
        if let Some(dev) = map.get_mut(&packet.device_id()) {
            if dev.addr.is_ipv6() && device_addr.is_ipv4() && IpSupport::ipv4() {
                // Prefer IPv4 address if we already have an IPv6 one for the same device ID.
                insert = true;
            } else if dev.addr.is_ipv4() == device_addr.is_ipv4()
                && dev.addr.is_ipv6() == device_addr.is_ipv6()
            {
                // Just update the time stamp for this device - it's still alive.
                dev.last_seen = Instant::now();
            }
        } else {
            insert = true;
        }
        if insert {
            map.insert(
                packet.device_id(),
                DiscoveredDevice {
                    device_id: packet.device_id(),
                    device_name: packet
                        .device_name
                        .as_str()
                        .context("Device name conversion failed")?
                        .to_string(),
                    addr: device_addr,
                    transfer_port: packet.transfer_port,
                    last_seen: Instant::now(),
                },
            );
        }
    }

    Ok(())
}

/// Waits for one discovery packet and processes it.
/// Returns `true` if the socket is healthy (packet received or ignored),
/// `false` on a socket-level I/O error (caller should recreate the socket).
pub async fn listen_for_devices(socket: &UdpSocket, own_id: Uuid, devices: &DeviceMap) -> bool {
    let mut buf = [0u8; DiscoveryPacket::size()];
    match socket.recv_from(&mut buf).await {
        Ok((len, addr)) => {
            match DiscoveryPacket::deserialize(&buf[..len]) {
                Ok(packet) if packet.device_id() != own_id => {
                    if !packet.verify_checksum() {
                        log::warn!(
                            "Discovery packet from {addr} failed checksum verification - discarding"
                        );
                        return true;
                    }
                    if let Err(e) = update_device(devices, packet, addr).await {
                        log::debug!("Failed to update device: {e}");
                    }
                }
                Ok(_packet) => (), // Ignore our own discovery packets
                Err(e) => {
                    log::debug!("Failed to deserialize discovery packet from {addr}: {e}");
                }
            }
            true
        }
        Err(e) => {
            log::debug!("Discovery recv error: {e}");
            false
        }
    }
}

pub async fn prune_stale_devices(devices: &DeviceMap) {
    let mut map = devices.lock().await;
    let now = Instant::now();
    map.retain(|_, dev| now.duration_since(dev.last_seen) < DEVICE_TIMEOUT);
}
