//! UPnP / NAT-PMP port mapping for home relays (same mechanism as BitTorrent clients).
//!
//! When `GHAL_BOL_RELAY_DYNAMIC=1`, the relay binds an ephemeral local TCP port and asks the
//! router to forward a WAN port to it. `GET /v1/relay` advertises the **external** port so
//! clients never hardcode 4002 on home installs.
//!
//! Remap policy (TRANSPORT.md § Event-driven async): startup worker retries until first map;
//! runtime remap only on client `GET /v1/relay?remap=true` after bootstrap failure (storm-throttled),
//! not on a periodic timer.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use igd_next::aio::Gateway;
use igd_next::aio::tokio::search_gateway;
use igd_next::GetGenericPortMappingEntryError;
use igd_next::PortMappingProtocol;
use igd_next::SearchOptions;

type UpnpGateway = Gateway<igd_next::aio::tokio::Tokio>;

const MAX_PORT_TABLE_SCAN: u32 = 128;
const MAP_VERIFY_ATTEMPTS: u32 = 3;

/// Result of a successful router port mapping.
#[derive(Clone, Debug)]
pub struct MappedPort {
    pub local_addr: SocketAddr,
    pub external_ip: IpAddr,
    pub external_port: u16,
}

impl MappedPort {
    /// `/ip4/<wan-ip>/tcp/<external-port>` — fallback when DNS lags DDNS.
    pub fn external_multiaddr(&self) -> Option<String> {
        match self.external_ip {
            IpAddr::V4(v4) => Some(format!("/ip4/{v4}/tcp/{}", self.external_port)),
            IpAddr::V6(v6) => Some(format!("/ip6/{v6}/tcp/{}", self.external_port)),
        }
    }

    /// Direct LAN dial to the relay listen socket — same-subnet when WAN hairpin fails.
    pub fn local_lan_multiaddr(&self) -> Option<String> {
        match self.local_addr.ip() {
            IpAddr::V4(v4) if v4.is_private() && !v4.is_loopback() => {
                Some(format!("/ip4/{v4}/tcp/{}", self.local_addr.port()))
            }
            _ => None,
        }
    }
}

/// Best-effort LAN IPv4 for UPnP internal address (UDP trick to default route).
pub fn guess_local_ipv4() -> std::io::Result<Ipv4Addr> {
    use std::net::UdpSocket;
    let sock = UdpSocket::bind("0.0.0.0:0")?;
    // Does not send traffic; picks the kernel's chosen egress interface.
    sock.connect("8.8.8.8:80")?;
    match sock.local_addr()?.ip() {
        IpAddr::V4(v4) => Ok(v4),
        IpAddr::V6(_) => Err(std::io::Error::new(
            std::io::ErrorKind::AddrNotAvailable,
            "no IPv4 default route for UPnP",
        )),
    }
}

/// Reserve an ephemeral TCP port on the same address family as `hint` (wildcard preserved).
pub async fn reserve_ephemeral_tcp_port(hint: SocketAddr) -> std::io::Result<SocketAddr> {
    use tokio::net::TcpListener;
    let bind = match hint.ip() {
        IpAddr::V4(_) => SocketAddr::from(([0, 0, 0, 0], 0)),
        IpAddr::V6(_) => SocketAddr::from(([0, 0, 0, 0, 0, 0, 0, 0], 0)),
    };
    let listener = TcpListener::bind(bind).await?;
    listener.local_addr()
}

fn search_options_for_lan(local_ip: Ipv4Addr) -> SearchOptions {
    SearchOptions {
        bind_addr: SocketAddr::new(IpAddr::V4(local_ip), 0),
        timeout: Some(Duration::from_secs(30)),
        single_search_timeout: Some(Duration::from_secs(10)),
        ..SearchOptions::default()
    }
}

/// Map `local_listen.port()` on this host to any free external TCP port via UPnP/NAT-PMP.
pub async fn map_relay_port(
    local_listen: SocketAddr,
) -> Result<MappedPort, Box<dyn std::error::Error + Send + Sync>> {
    let local_ip = guess_local_ipv4()?;
    let local_addr = SocketAddr::new(IpAddr::V4(local_ip), local_listen.port());
    let gateway = search_gateway(search_options_for_lan(local_ip)).await?;
    tracing::info!(gateway = %gateway, %local_addr, "relay UPnP — gateway found");
    allocate_verified_port(&gateway, local_addr, None).await
}

/// Remap after a client bootstrap TCP failure on the currently advertised WAN port.
///
/// The failing port is **proven stale** — remove it and allocate a fresh external port.
/// Never "renew" the same external port on this signal (routers often ACK renew while
/// the forward rule is still dead).
pub async fn remap_after_client_bootstrap_failure(
    local_listen: SocketAddr,
    stale_external_port: Option<u16>,
) -> Result<MappedPort, Box<dyn std::error::Error + Send + Sync>> {
    let local_ip = guess_local_ipv4()?;
    let local_addr = SocketAddr::new(IpAddr::V4(local_ip), local_listen.port());
    let gateway = search_gateway(search_options_for_lan(local_ip)).await?;
    tracing::info!(gateway = %gateway, %local_addr, "relay UPnP — gateway found");
    if let Some(ext) = stale_external_port {
        if let Err(e) = gateway.remove_port(PortMappingProtocol::TCP, ext).await {
            tracing::debug!(
                external_port = ext,
                error = %e,
                "relay UPnP remove stale external port (best-effort)"
            );
        } else {
            tracing::info!(
                external_port = ext,
                "relay UPnP removed stale external port after client bootstrap failure"
            );
        }
    }
    allocate_verified_port(&gateway, local_addr, stale_external_port).await
}

async fn allocate_verified_port(
    gateway: &UpnpGateway,
    local_addr: SocketAddr,
    exclude_external_port: Option<u16>,
) -> Result<MappedPort, Box<dyn std::error::Error + Send + Sync>> {
    let mut last_err: Option<Box<dyn std::error::Error + Send + Sync>> = None;
    for attempt in 1..=MAP_VERIFY_ATTEMPTS {
        match gateway
            .get_any_address(
                PortMappingProtocol::TCP,
                local_addr,
                0,
                "ghal_bol relay",
            )
            .await
        {
            Ok(external) => {
                let mapping = MappedPort {
                    local_addr,
                    external_ip: external.ip(),
                    external_port: external.port(),
                };
                if exclude_external_port == Some(mapping.external_port) {
                    let _ = gateway
                        .remove_port(PortMappingProtocol::TCP, mapping.external_port)
                        .await;
                    last_err = Some("router reused stale external port".into());
                    continue;
                }
                match verify_mapping_on_gateway(gateway, &mapping).await {
                    Ok(()) => {
                        tracing::info!(
                            external_port = mapping.external_port,
                            external = %mapping.external_ip,
                            %local_addr,
                            attempt,
                            "relay UPnP mapped and verified"
                        );
                        return Ok(mapping);
                    }
                    Err(e) => {
                        tracing::warn!(
                            external_port = mapping.external_port,
                            error = %e,
                            attempt,
                            "relay UPnP mapping verify failed — removing and retrying"
                        );
                        let _ = gateway
                            .remove_port(PortMappingProtocol::TCP, mapping.external_port)
                            .await;
                        last_err = Some(e.into());
                    }
                }
            }
            Err(e) => {
                tracing::warn!(attempt, error = %e, "relay UPnP get_any_address failed");
                last_err = Some(format!("{e}").into());
            }
        }
    }
    Err(last_err.unwrap_or_else(|| "UPnP map failed".into()))
}

/// Read the router port table and confirm the external port forwards to our listen socket.
async fn verify_mapping_on_gateway(
    gateway: &UpnpGateway,
    expected: &MappedPort,
) -> Result<(), String> {
    let local_ip = match expected.local_addr.ip() {
        IpAddr::V4(v4) => v4,
        _ => return Err("UPnP verify requires IPv4 local address".into()),
    };

    for index in 0..MAX_PORT_TABLE_SCAN {
        match gateway.get_generic_port_mapping_entry(index).await {
            Ok(entry) => {
                if entry.protocol != PortMappingProtocol::TCP
                    || entry.external_port != expected.external_port
                {
                    continue;
                }
                if !entry.enabled {
                    return Err(format!(
                        "UPnP ext {} disabled on router",
                        expected.external_port
                    ));
                }
                let client_v4 = entry
                    .internal_client
                    .parse::<Ipv4Addr>()
                    .map_err(|_| format!("invalid internal_client {}", entry.internal_client))?;
                if client_v4 != local_ip {
                    return Err(format!(
                        "UPnP ext {} forwards to {client_v4}, expected {local_ip}",
                        expected.external_port
                    ));
                }
                if entry.internal_port != expected.local_addr.port() {
                    return Err(format!(
                        "UPnP ext {} internal port {} != listen {}",
                        expected.external_port,
                        entry.internal_port,
                        expected.local_addr.port()
                    ));
                }
                return Ok(());
            }
            Err(GetGenericPortMappingEntryError::SpecifiedArrayIndexInvalid) => {
                // Router exposes no port table — trust add_port success.
                tracing::debug!(
                    external_port = expected.external_port,
                    "relay UPnP verify skipped — router has no port table API"
                );
                return Ok(());
            }
            Err(e) => {
                tracing::debug!(
                    index,
                    error = %e,
                    "relay UPnP port table scan stopped"
                );
                break;
            }
        }
    }
    Err(format!(
        "UPnP ext {} not found in router port table",
        expected.external_port
    ))
}

/// Retry UPnP until the router responds (home routers can be slow right after boot).
pub async fn map_relay_port_with_retries(
    local_listen: SocketAddr,
    attempts: u32,
) -> Result<MappedPort, Box<dyn std::error::Error + Send + Sync>> {
    let mut last_err: Option<Box<dyn std::error::Error + Send + Sync>> = None;
    for attempt in 1..=attempts {
        match map_relay_port(local_listen).await {
            Ok(m) => return Ok(m),
            Err(e) => {
                tracing::debug!(attempt, error = %e, "relay UPnP attempt failed");
                last_err = Some(e);
                if attempt < attempts {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }
    }
    Err(last_err.unwrap_or_else(|| "UPnP failed".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mapped_port_external_multiaddr_ipv4() {
        let m = MappedPort {
            local_addr: "192.168.1.38:45123".parse().unwrap(),
            external_ip: "117.212.85.107".parse().unwrap(),
            external_port: 51234,
        };
        assert_eq!(
            m.external_multiaddr().as_deref(),
            Some("/ip4/117.212.85.107/tcp/51234")
        );
        assert_eq!(
            m.local_lan_multiaddr().as_deref(),
            Some("/ip4/192.168.1.38/tcp/45123")
        );
    }
}
