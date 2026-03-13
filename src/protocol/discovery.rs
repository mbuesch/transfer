use crate::protocol::packets::{DEVICE_TIMEOUT, DISCOVERY_PORT, DiscoveredDevice, DiscoveryPacket};
use socket2::{Domain, Protocol, Socket, Type};
use std::{
    collections::{BTreeSet, HashMap},
    net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6},
    sync::Arc,
    time::Instant,
};
use tokio::{net::UdpSocket, sync::Mutex};

const IPV6_MULTICAST_ADDR: Ipv6Addr = Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0, 0, 0x0001);

fn non_loopback_ipv6_if_indices() -> Vec<u32> {
    let Ok(content) = std::fs::read_to_string("/proc/net/if_inet6") else {
        return vec![];
    };
    let mut indices = BTreeSet::new();
    for line in content.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        // Format: <addr> <if_index_hex> <prefix_len> <scope> <flags> <if_name>
        if parts.len() >= 6
            && parts[5] != "lo"
            && let Ok(idx) = u32::from_str_radix(parts[1], 16)
        {
            indices.insert(idx);
        }
    }
    indices.into_iter().collect()
}

pub type DeviceMap = Arc<Mutex<HashMap<String, DiscoveredDevice>>>;

pub async fn create_ipv4_broadcast_socket() -> std::io::Result<UdpSocket> {
    let socket = tokio::net::UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)).await?;
    socket.set_broadcast(true)?;
    Ok(socket)
}

pub async fn create_ipv4_listener_socket() -> std::io::Result<UdpSocket> {
    let socket =
        tokio::net::UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, DISCOVERY_PORT))
            .await?;
    socket.set_broadcast(true)?;
    Ok(socket)
}

pub fn create_ipv6_sender_socket() -> std::io::Result<UdpSocket> {
    let sock = Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP))?;
    sock.set_only_v6(true)?;
    sock.set_nonblocking(true)?;
    sock.bind(&SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 0, 0, 0).into())?;
    let std_sock: std::net::UdpSocket = sock.into();
    UdpSocket::from_std(std_sock)
}

pub fn create_ipv6_listener_socket() -> std::io::Result<UdpSocket> {
    let sock = Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP))?;
    sock.set_only_v6(true)?;
    sock.set_reuse_address(true)?;
    sock.set_nonblocking(true)?;
    sock.bind(&SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, DISCOVERY_PORT, 0, 0).into())?;
    let std_sock: std::net::UdpSocket = sock.into();
    let udp = UdpSocket::from_std(std_sock)?;
    for idx in non_loopback_ipv6_if_indices() {
        if let Err(e) = udp.join_multicast_v6(&IPV6_MULTICAST_ADDR, idx) {
            log::debug!("Failed to join IPv6 multicast on interface {idx}: {e}");
        }
    }
    Ok(udp)
}

pub async fn broadcast_presence(socket: &UdpSocket, packet: &DiscoveryPacket) {
    let data = match serde_json::to_vec(packet) {
        Ok(d) => d,
        Err(e) => {
            log::error!("Failed to serialize discovery packet: {e}");
            return;
        }
    };
    let broadcast_addr = SocketAddrV4::new(Ipv4Addr::BROADCAST, DISCOVERY_PORT);
    if let Err(e) = socket.send_to(&data, broadcast_addr).await {
        log::debug!("Broadcast send error (non-fatal): {e}");
    }
}

pub async fn broadcast_presence_v6(socket: &UdpSocket, packet: &DiscoveryPacket) {
    let data = match serde_json::to_vec(packet) {
        Ok(d) => d,
        Err(e) => {
            log::error!("Failed to serialize IPv6 discovery packet: {e}");
            return;
        }
    };
    for idx in non_loopback_ipv6_if_indices() {
        let addr = SocketAddrV6::new(IPV6_MULTICAST_ADDR, DISCOVERY_PORT, 0, idx);
        if let Err(e) = socket.send_to(&data, addr).await {
            log::debug!("IPv6 multicast send error on interface {idx} (non-fatal): {e}");
        }
    }
}

async fn update_device(devices: &DeviceMap, packet: DiscoveryPacket, addr: SocketAddr) {
    let device_addr = match addr {
        SocketAddr::V6(v6) => SocketAddr::V6(SocketAddrV6::new(
            *v6.ip(),
            packet.transfer_port,
            v6.flowinfo(),
            v6.scope_id(),
        )),
        SocketAddr::V4(_) => SocketAddr::new(addr.ip(), packet.transfer_port),
    };

    {
        let mut map = devices.lock().await;

        let mut insert = false;
        if let Some(dev) = map.get_mut(&packet.device_id) {
            if dev.addr.is_ipv6() && device_addr.is_ipv4() {
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
                packet.device_id.clone(),
                DiscoveredDevice {
                    device_id: packet.device_id,
                    device_name: packet.device_name,
                    addr: device_addr,
                    transfer_port: packet.transfer_port,
                    last_seen: Instant::now(),
                },
            );
        }
    }
}

pub async fn listen_for_devices(socket: &UdpSocket, own_id: &str, devices: &DeviceMap) {
    let mut buf = [0u8; 4096];
    match socket.recv_from(&mut buf).await {
        Ok((len, addr)) => {
            if let Ok(packet) = serde_json::from_slice::<DiscoveryPacket>(&buf[..len])
                && packet.device_id != own_id
            {
                update_device(devices, packet, addr).await;
            }
        }
        Err(e) => {
            log::debug!("Discovery recv error: {e}");
        }
    }
}

pub async fn prune_stale_devices(devices: &DeviceMap) {
    let mut map = devices.lock().await;
    let now = Instant::now();
    map.retain(|_, dev| now.duration_since(dev.last_seen) < DEVICE_TIMEOUT);
}
