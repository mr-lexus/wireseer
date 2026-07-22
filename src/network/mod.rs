pub mod vendor;

use std::{
    collections::BTreeSet,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use chrono::Utc;
use futures::{StreamExt, stream};
use hickory_resolver::TokioAsyncResolver;
use if_addrs::{IfAddr, get_if_addrs};
use ipnet::Ipv4Net;
use serde::{Deserialize, Serialize};
use socket2::{Domain, Protocol, Socket, Type};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::mpsc,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    config::{Config, ScanMode},
    devices::{DiscoverySource, HttpMetadata, Observation},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkInterface {
    pub name: String,
    pub address: Ipv4Addr,
    pub netmask: Ipv4Addr,
    pub subnet: Ipv4Net,
    pub is_loopback: bool,
    pub likely_active: bool,
}

#[derive(Debug, Clone)]
pub struct ScanSpec {
    pub id: Uuid,
    pub interface: NetworkInterface,
    pub subnet: Ipv4Net,
    pub mode: ScanMode,
    pub local: bool,
    pub ports: Vec<u16>,
    pub connect_timeout: Duration,
    pub concurrency: usize,
    pub host_limit: usize,
    pub dns_timeout: Duration,
    pub reverse_dns: bool,
    pub ssdp: bool,
    pub arp: bool,
    pub mdns: bool,
    pub http_metadata: bool,
}

impl ScanSpec {
    pub fn from_config(config: &Config, interface: NetworkInterface) -> Result<Self> {
        let subnet = config
            .subnet
            .as_ref()
            .map_or(Ok(interface.subnet), |value| {
                value.parse::<Ipv4Net>().context("parse selected subnet")
            })?;
        let host_count = subnet
            .hosts()
            .take(config.host_limit.saturating_add(1))
            .count();
        anyhow::ensure!(
            host_count <= config.host_limit,
            "subnet {subnet} contains more than the configured {} host safety limit",
            config.host_limit
        );
        Ok(Self {
            id: Uuid::new_v4(),
            interface,
            subnet,
            mode: config.scan_mode,
            local: config.enabled_protocols.local,
            ports: if config.enabled_protocols.tcp {
                config.scan_mode.ports().to_vec()
            } else {
                Vec::new()
            },
            connect_timeout: config.connect_timeout(),
            concurrency: config.concurrency,
            host_limit: config.host_limit,
            dns_timeout: Duration::from_millis(config.dns_timeout_ms),
            reverse_dns: config.enabled_protocols.reverse_dns,
            ssdp: config.enabled_protocols.ssdp,
            arp: config.enabled_protocols.arp,
            mdns: config.enabled_protocols.mdns,
            http_metadata: config.http_metadata,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanPhase {
    Preparing,
    Discovering,
    Enriching,
    Finishing,
    Complete,
    Cancelled,
}

impl ScanPhase {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Preparing => "Preparing",
            Self::Discovering => "Discovering hosts",
            Self::Enriching => "Enriching devices",
            Self::Finishing => "Recording changes",
            Self::Complete => "Complete",
            Self::Cancelled => "Cancelled",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ScanProgress {
    pub scan_id: Uuid,
    pub phase: ScanPhase,
    pub completed: usize,
    pub total: usize,
    pub active: usize,
    pub devices_found: usize,
}

impl ScanProgress {
    #[must_use]
    pub fn ratio(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.completed as f64 / self.total as f64
        }
    }
}

#[derive(Debug, Clone)]
pub enum DiscoveryEvent {
    Started {
        spec: ScanSpec,
        started_at: chrono::DateTime<Utc>,
    },
    Observation(Observation),
    Progress(ScanProgress),
    ProviderHealth(ProviderHealth),
    Finished {
        scan_id: Uuid,
        cancelled: bool,
    },
    Failed {
        scan_id: Uuid,
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthStatus {
    Available,
    Disabled,
    Degraded,
    Unavailable,
}

#[derive(Debug, Clone)]
pub struct ProviderHealth {
    pub provider: &'static str,
    pub status: HealthStatus,
    pub detail: String,
}

pub fn interfaces() -> Result<Vec<NetworkInterface>> {
    let entries = get_if_addrs().context("enumerate local interfaces")?;
    let mut seen = BTreeSet::new();
    let mut interfaces = Vec::new();
    for entry in entries {
        let IfAddr::V4(v4) = entry.addr else { continue };
        if !seen.insert((entry.name.clone(), v4.ip)) {
            continue;
        }
        let prefix = netmask_prefix(v4.netmask);
        let Ok(subnet) = Ipv4Net::new(network_address(v4.ip, prefix), prefix) else {
            continue;
        };
        let is_loopback =
            v4.ip.is_loopback() || entry.name == "lo" || entry.name.starts_with("lo:");
        let likely_active = !is_loopback && !v4.ip.is_link_local() && !v4.ip.is_unspecified();
        interfaces.push(NetworkInterface {
            name: entry.name,
            address: v4.ip,
            netmask: v4.netmask,
            subnet,
            is_loopback,
            likely_active,
        });
    }
    interfaces.sort_by_key(|interface| (!interface.likely_active, interface.name.clone()));
    Ok(interfaces)
}

pub fn select_interface(
    available: &[NetworkInterface],
    preferred: Option<&str>,
) -> Result<NetworkInterface> {
    if let Some(name) = preferred {
        return available
            .iter()
            .find(|interface| interface.name == name)
            .cloned()
            .with_context(|| format!("interface '{name}' has no IPv4 address"));
    }
    if let Some(route) = default_route() {
        if let Some(interface) = available
            .iter()
            .find(|interface| interface.name == route.interface)
        {
            return Ok(interface.clone());
        }
    }
    // Prefer conventional Ethernet/Wi-Fi names over container bridges and
    // virtual adapters when a platform does not expose its routing table.
    if let Some(interface) = available
        .iter()
        .find(|interface| interface.likely_active && !is_virtual_interface(&interface.name))
    {
        return Ok(interface.clone());
    }
    available
        .iter()
        .find(|interface| interface.likely_active)
        .or_else(|| available.iter().find(|interface| !interface.is_loopback))
        .or_else(|| available.first())
        .cloned()
        .context("no IPv4 network interface is available")
}

fn is_virtual_interface(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with("br-")
        || lower.starts_with("docker")
        || lower.starts_with("veth")
        || lower.starts_with("virbr")
        || lower.starts_with("cni")
        || lower.starts_with("podman")
        || lower.starts_with("tailscale")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefaultRoute {
    pub interface: String,
    pub gateway: Ipv4Addr,
}

#[must_use]
#[cfg(target_os = "linux")]
pub fn default_route() -> Option<DefaultRoute> {
    let contents = std::fs::read_to_string("/proc/net/route").ok()?;
    parse_default_route(&contents)
}

#[must_use]
#[cfg(not(target_os = "linux"))]
pub fn default_route() -> Option<DefaultRoute> {
    None
}

#[cfg(any(target_os = "linux", test))]
fn parse_default_route(contents: &str) -> Option<DefaultRoute> {
    contents
        .lines()
        .skip(1)
        .filter_map(|line| {
            let fields = line.split_whitespace().collect::<Vec<_>>();
            if fields.len() <= 6 || fields[1] != "00000000" {
                return None;
            }
            let flags = u16::from_str_radix(fields[3], 16).ok()?;
            if flags & 0x3 != 0x3 {
                return None;
            }
            let gateway = u32::from_str_radix(fields[2], 16).ok()?;
            let metric = fields[6].parse::<u32>().ok()?;
            Some((
                metric,
                DefaultRoute {
                    interface: fields[0].to_string(),
                    gateway: Ipv4Addr::from(gateway.to_le_bytes()),
                },
            ))
        })
        .min_by_key(|(metric, _)| *metric)
        .map(|(_, route)| route)
}

#[must_use]
pub fn provider_capabilities(config: &Config) -> Vec<ProviderHealth> {
    let tcp_active = config.enabled_protocols.tcp && config.scan_mode != ScanMode::Passive;
    vec![
        ProviderHealth {
            provider: "local",
            status: if config.enabled_protocols.local {
                HealthStatus::Available
            } else {
                HealthStatus::Disabled
            },
            detail: if config.enabled_protocols.local {
                "portable interface enumeration"
            } else {
                "disabled in configuration"
            }
            .into(),
        },
        ProviderHealth {
            provider: "tcp",
            status: if tcp_active {
                HealthStatus::Available
            } else {
                HealthStatus::Disabled
            },
            detail: if tcp_active {
                "bounded TCP connect checks"
            } else if !config.enabled_protocols.tcp {
                "disabled in configuration"
            } else {
                "inactive in passive scan mode"
            }
            .into(),
        },
        ProviderHealth {
            provider: "reverse-dns",
            status: if config.enabled_protocols.reverse_dns {
                HealthStatus::Available
            } else {
                HealthStatus::Disabled
            },
            detail: if config.enabled_protocols.reverse_dns {
                "bounded reverse lookups through the system resolver"
            } else {
                "disabled in configuration"
            }
            .into(),
        },
        ProviderHealth {
            provider: "arp",
            status: if config.enabled_protocols.arp && cfg!(target_os = "linux") {
                HealthStatus::Available
            } else if !config.enabled_protocols.arp {
                HealthStatus::Disabled
            } else {
                HealthStatus::Unavailable
            },
            detail: if !config.enabled_protocols.arp {
                "disabled in configuration"
            } else if cfg!(target_os = "linux") {
                "Linux neighbor cache (passive, no raw socket required)"
            } else {
                "portable neighbor-cache adapter unavailable; TCP discovery continues"
            }
            .into(),
        },
        ProviderHealth {
            provider: "icmp",
            status: if config.enabled_protocols.icmp {
                HealthStatus::Unavailable
            } else {
                HealthStatus::Disabled
            },
            detail: if config.enabled_protocols.icmp {
                "raw ICMP adapter unavailable; a host is never marked offline for this reason"
            } else {
                "disabled in configuration"
            }
            .into(),
        },
        ProviderHealth {
            provider: "ssdp",
            status: if config.enabled_protocols.ssdp {
                HealthStatus::Available
            } else {
                HealthStatus::Disabled
            },
            detail: if config.enabled_protocols.ssdp {
                "one bounded UPnP discovery request per scan"
            } else {
                "disabled in configuration"
            }
            .into(),
        },
        ProviderHealth {
            provider: "mdns",
            status: if config.enabled_protocols.mdns {
                HealthStatus::Available
            } else {
                HealthStatus::Disabled
            },
            detail: if config.enabled_protocols.mdns {
                "one bounded DNS-SD service enumeration query per scan"
            } else {
                "disabled in configuration"
            }
            .into(),
        },
        ProviderHealth {
            provider: "netbios",
            status: if config.enabled_protocols.netbios {
                HealthStatus::Unavailable
            } else {
                HealthStatus::Disabled
            },
            detail: if config.enabled_protocols.netbios {
                "optional provider is not enabled in this build"
            } else {
                "disabled in configuration"
            }
            .into(),
        },
        ProviderHealth {
            provider: "http-metadata",
            status: if config.http_metadata && tcp_active {
                HealthStatus::Available
            } else {
                HealthStatus::Disabled
            },
            detail: if config.http_metadata && tcp_active {
                "single bounded GET /; no authentication, crawling, or redirects"
            } else if !config.http_metadata {
                "disabled in configuration"
            } else {
                "inactive because TCP discovery is inactive"
            }
            .into(),
        },
        ProviderHealth {
            provider: "tls-metadata",
            status: if config.tls_metadata {
                HealthStatus::Unavailable
            } else {
                HealthStatus::Disabled
            },
            detail: if config.tls_metadata {
                "certificate inspection is unavailable in this build"
            } else {
                "disabled in configuration"
            }
            .into(),
        },
    ]
}

pub fn spawn_scan(
    spec: ScanSpec,
    sender: mpsc::Sender<DiscoveryEvent>,
    cancellation: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let id = spec.id;
        if let Err(error) = run_scan(spec, sender.clone(), cancellation).await {
            let _ = sender
                .send(DiscoveryEvent::Failed {
                    scan_id: id,
                    message: format!("{error:#}"),
                })
                .await;
        }
    })
}

/// Deterministic event source for UI/integration tests and recorded demos. It
/// performs no network operations and follows the same event contract as real providers.
pub fn spawn_mock_scan(
    spec: ScanSpec,
    observations: Vec<Observation>,
    sender: mpsc::Sender<DiscoveryEvent>,
    cancellation: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let started_at = Utc::now();
        let total = observations.len();
        let _ = sender
            .send(DiscoveryEvent::Started {
                spec: spec.clone(),
                started_at,
            })
            .await;
        for (index, observation) in observations.into_iter().enumerate() {
            if cancellation.is_cancelled() {
                let _ = sender
                    .send(DiscoveryEvent::Finished {
                        scan_id: spec.id,
                        cancelled: true,
                    })
                    .await;
                return;
            }
            if sender
                .send(DiscoveryEvent::Observation(observation))
                .await
                .is_err()
            {
                return;
            }
            let _ = sender
                .send(DiscoveryEvent::Progress(ScanProgress {
                    scan_id: spec.id,
                    phase: ScanPhase::Discovering,
                    completed: index + 1,
                    total,
                    active: 0,
                    devices_found: index + 1,
                }))
                .await;
            tokio::task::yield_now().await;
        }
        let _ = sender
            .send(DiscoveryEvent::Finished {
                scan_id: spec.id,
                cancelled: false,
            })
            .await;
    })
}

async fn run_scan(
    spec: ScanSpec,
    sender: mpsc::Sender<DiscoveryEvent>,
    cancellation: CancellationToken,
) -> Result<()> {
    let started_at = Utc::now();
    sender
        .send(DiscoveryEvent::Started {
            spec: spec.clone(),
            started_at,
        })
        .await
        .context("send scan start")?;
    if spec.local {
        let local = Observation {
            ip: spec.interface.address,
            mac: None,
            hostname: Some("This computer".into()),
            vendor: None,
            open_port: None,
            latency: Some(Duration::ZERO),
            source: DiscoverySource::Local,
            interface: spec.interface.name.clone(),
            subnet: spec.subnet.to_string(),
            observed_at: Utc::now(),
            http: None,
        };
        sender.send(DiscoveryEvent::Observation(local)).await?;
    }

    let hosts: Vec<Ipv4Addr> = if spec.ports.is_empty() {
        Vec::new()
    } else {
        spec.subnet.hosts().take(spec.host_limit).collect()
    };
    let total = hosts.len().saturating_mul(spec.ports.len());
    let completed = Arc::new(AtomicUsize::new(0));
    let found = Arc::new(AtomicUsize::new(usize::from(spec.local)));
    let active = Arc::new(AtomicUsize::new(0));
    let interface = spec.interface.name.clone();
    let subnet = spec.subnet.to_string();
    let timeout = spec.connect_timeout;
    let http_metadata = spec.http_metadata;
    let progress_sender = sender.clone();
    let progress_cancel = cancellation.clone();
    let progress_completed = Arc::clone(&completed);
    let progress_found = Arc::clone(&found);
    let progress_active = Arc::clone(&active);
    let scan_id = spec.id;
    let progress_task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(120));
        loop {
            tokio::select! {
                () = progress_cancel.cancelled() => break,
                _ = interval.tick() => {
                    let done = progress_completed.load(Ordering::Relaxed);
                    if progress_sender.send(DiscoveryEvent::Progress(ScanProgress {
                        scan_id,
                        phase: ScanPhase::Discovering,
                        completed: done,
                        total,
                        active: progress_active.load(Ordering::Relaxed),
                        devices_found: progress_found.load(Ordering::Relaxed),
                    })).await.is_err() || done >= total {
                        break;
                    }
                }
            }
        }
    });

    let jobs = hosts
        .into_iter()
        .flat_map(|ip| spec.ports.iter().copied().map(move |port| (ip, port)))
        .collect::<Vec<_>>();
    let mut checks = stream::iter(jobs)
        .map(|(ip, port)| {
            let cancellation = cancellation.clone();
            let completed = Arc::clone(&completed);
            let active = Arc::clone(&active);
            async move {
                active.fetch_add(1, Ordering::Relaxed);
                let started = Instant::now();
                let address = SocketAddr::new(IpAddr::V4(ip), port);
                let result = tokio::select! {
                    () = cancellation.cancelled() => None,
                    value = tokio::time::timeout(timeout, TcpStream::connect(address)) => {
                        match value {
                            Ok(Ok(mut stream)) => {
                                let latency = started.elapsed();
                                let http = if http_metadata && matches!(port, 80 | 8008 | 8080) {
                                    safe_http_metadata(&mut stream, ip, timeout, &cancellation).await
                                } else {
                                    None
                                };
                                Some((ip, port, latency, http))
                            }
                            Ok(Err(_)) | Err(_) => None,
                        }
                    }
                };
                active.fetch_sub(1, Ordering::Relaxed);
                completed.fetch_add(1, Ordering::Relaxed);
                result
            }
        })
        .buffer_unordered(spec.concurrency.max(1));

    let mut found_ips = BTreeSet::new();
    while let Some(result) = checks.next().await {
        if cancellation.is_cancelled() {
            break;
        }
        if let Some((ip, port, latency, http)) = result {
            if found_ips.insert(ip) {
                found.fetch_add(1, Ordering::Relaxed);
            }
            let mut observation = Observation::tcp(ip, port, latency, &interface, &subnet);
            observation.http = http;
            sender
                .send(DiscoveryEvent::Observation(observation))
                .await?;
        }
    }
    if !cancellation.is_cancelled() && spec.arp {
        discover_neighbor_cache(&spec, &sender, &mut found_ips).await;
    }
    if !cancellation.is_cancelled() && spec.mdns {
        discover_mdns(&spec, &sender, &cancellation, &mut found_ips).await;
    }
    if !cancellation.is_cancelled() && spec.ssdp {
        discover_ssdp(&spec, &sender, &cancellation, &mut found_ips).await;
    }
    if !cancellation.is_cancelled() && spec.reverse_dns && !found_ips.is_empty() {
        let _ = sender
            .send(DiscoveryEvent::Progress(ScanProgress {
                scan_id: spec.id,
                phase: ScanPhase::Enriching,
                completed: 0,
                total: found_ips.len(),
                active: 0,
                devices_found: found.load(Ordering::Relaxed),
            }))
            .await;
        enrich_reverse_dns(&spec, &sender, &cancellation, found_ips.iter().copied()).await;
    }
    progress_task.abort();
    let cancelled = cancellation.is_cancelled();
    sender
        .send(DiscoveryEvent::Progress(ScanProgress {
            scan_id: spec.id,
            phase: if cancelled {
                ScanPhase::Cancelled
            } else {
                ScanPhase::Complete
            },
            completed: completed.load(Ordering::Relaxed),
            total,
            active: 0,
            devices_found: found.load(Ordering::Relaxed),
        }))
        .await?;
    sender
        .send(DiscoveryEvent::Finished {
            scan_id: spec.id,
            cancelled,
        })
        .await?;
    Ok(())
}

#[cfg(target_os = "linux")]
async fn discover_neighbor_cache(
    spec: &ScanSpec,
    sender: &mpsc::Sender<DiscoveryEvent>,
    found_ips: &mut BTreeSet<Ipv4Addr>,
) {
    let Ok(contents) = tokio::fs::read_to_string("/proc/net/arp").await else {
        let _ = sender
            .send(DiscoveryEvent::ProviderHealth(ProviderHealth {
                provider: "arp",
                status: HealthStatus::Degraded,
                detail: "Linux neighbor cache could not be read".into(),
            }))
            .await;
        return;
    };
    for line in contents.lines().skip(1) {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 6 || fields[5] != spec.interface.name {
            continue;
        }
        let Ok(ip) = fields[0].parse::<Ipv4Addr>() else {
            continue;
        };
        if !spec.subnet.contains(&ip) || fields[3] == "00:00:00:00:00:00" {
            continue;
        }
        found_ips.insert(ip);
        let observation = Observation {
            ip,
            mac: Some(fields[3].to_ascii_uppercase()),
            hostname: None,
            vendor: None,
            open_port: None,
            latency: None,
            source: DiscoverySource::Arp,
            interface: spec.interface.name.clone(),
            subnet: spec.subnet.to_string(),
            observed_at: Utc::now(),
            http: None,
        };
        if sender
            .send(DiscoveryEvent::Observation(observation))
            .await
            .is_err()
        {
            break;
        }
    }
}

#[cfg(not(target_os = "linux"))]
async fn discover_neighbor_cache(
    _spec: &ScanSpec,
    sender: &mpsc::Sender<DiscoveryEvent>,
    _found_ips: &mut BTreeSet<Ipv4Addr>,
) {
    let _ = sender
        .send(DiscoveryEvent::ProviderHealth(ProviderHealth {
            provider: "arp",
            status: HealthStatus::Unavailable,
            detail: "neighbor-cache access is unsupported on this platform".into(),
        }))
        .await;
}

async fn discover_mdns(
    spec: &ScanSpec,
    sender: &mpsc::Sender<DiscoveryEvent>,
    cancellation: &CancellationToken,
    found_ips: &mut BTreeSet<Ipv4Addr>,
) {
    let socket = match mdns_socket() {
        Ok(socket) => socket,
        Err(error) => {
            let _ = sender
                .send(DiscoveryEvent::ProviderHealth(ProviderHealth {
                    provider: "mdns",
                    status: HealthStatus::Degraded,
                    detail: format!("multicast socket unavailable: {error}"),
                }))
                .await;
            return;
        }
    };
    // Standard DNS-SD meta-query: PTR _services._dns-sd._udp.local.
    const QUERY: &[u8] = &[
        0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 9, b'_', b's', b'e', b'r', b'v', b'i', b'c', b'e',
        b's', 7, b'_', b'd', b'n', b's', b'-', b's', b'd', 4, b'_', b'u', b'd', b'p', 5, b'l',
        b'o', b'c', b'a', b'l', 0, 0, 12, 0, 1,
    ];
    if let Err(error) = socket
        .send_to(QUERY, (Ipv4Addr::new(224, 0, 0, 251), 5353))
        .await
    {
        let _ = sender
            .send(DiscoveryEvent::ProviderHealth(ProviderHealth {
                provider: "mdns",
                status: HealthStatus::Degraded,
                detail: format!("multicast query failed: {error}"),
            }))
            .await;
        return;
    }
    let deadline = tokio::time::Instant::now() + Duration::from_millis(850);
    let mut buffer = [0_u8; 9_000];
    loop {
        let received = tokio::select! {
            () = cancellation.cancelled() => return,
            result = tokio::time::timeout_at(deadline, socket.recv_from(&mut buffer)) => result,
        };
        let Ok(Ok((length, source))) = received else {
            break;
        };
        if length < 12 {
            continue;
        }
        let IpAddr::V4(ip) = source.ip() else {
            continue;
        };
        if !spec.subnet.contains(&ip) || ip == spec.interface.address {
            continue;
        }
        found_ips.insert(ip);
        let observation = Observation {
            ip,
            mac: None,
            hostname: None,
            vendor: None,
            open_port: None,
            latency: None,
            source: DiscoverySource::Mdns,
            interface: spec.interface.name.clone(),
            subnet: spec.subnet.to_string(),
            observed_at: Utc::now(),
            http: None,
        };
        if sender
            .send(DiscoveryEvent::Observation(observation))
            .await
            .is_err()
        {
            break;
        }
    }
}

fn mdns_socket() -> Result<tokio::net::UdpSocket> {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))
        .context("create mDNS socket")?;
    socket
        .set_reuse_address(true)
        .context("enable mDNS address reuse")?;
    socket
        .bind(&SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 5353).into())
        .context("bind mDNS port 5353")?;
    socket
        .join_multicast_v4(&Ipv4Addr::new(224, 0, 0, 251), &Ipv4Addr::UNSPECIFIED)
        .context("join mDNS multicast group")?;
    socket.set_nonblocking(true)?;
    tokio::net::UdpSocket::from_std(socket.into()).context("register mDNS socket")
}

async fn enrich_reverse_dns(
    spec: &ScanSpec,
    sender: &mpsc::Sender<DiscoveryEvent>,
    cancellation: &CancellationToken,
    ips: impl Iterator<Item = Ipv4Addr>,
) {
    let resolver = match TokioAsyncResolver::tokio_from_system_conf() {
        Ok(resolver) => Arc::new(resolver),
        Err(error) => {
            let _ = sender
                .send(DiscoveryEvent::ProviderHealth(ProviderHealth {
                    provider: "reverse-dns",
                    status: HealthStatus::Degraded,
                    detail: format!("system resolver unavailable: {error}"),
                }))
                .await;
            return;
        }
    };
    let interface = spec.interface.name.clone();
    let subnet = spec.subnet.to_string();
    let timeout = spec.dns_timeout;
    let mut lookups = stream::iter(ips)
        .map(|ip| {
            let resolver = Arc::clone(&resolver);
            let cancellation = cancellation.clone();
            async move {
                tokio::select! {
                    () = cancellation.cancelled() => None,
                    result = tokio::time::timeout(timeout, resolver.reverse_lookup(IpAddr::V4(ip))) => {
                        result.ok().and_then(Result::ok).and_then(|names| {
                            names.iter().next().map(|name| {
                                (ip, name.to_utf8().trim_end_matches('.').to_string())
                            })
                        })
                    }
                }
            }
        })
        .buffer_unordered(spec.concurrency.clamp(1, 32));
    while let Some(Some((ip, hostname))) = lookups.next().await {
        let observation = Observation {
            ip,
            mac: None,
            hostname: Some(hostname),
            vendor: None,
            open_port: None,
            latency: None,
            source: DiscoverySource::ReverseDns,
            interface: interface.clone(),
            subnet: subnet.clone(),
            observed_at: Utc::now(),
            http: None,
        };
        if sender
            .send(DiscoveryEvent::Observation(observation))
            .await
            .is_err()
        {
            break;
        }
    }
}

async fn discover_ssdp(
    spec: &ScanSpec,
    sender: &mpsc::Sender<DiscoveryEvent>,
    cancellation: &CancellationToken,
    found_ips: &mut BTreeSet<Ipv4Addr>,
) {
    const REQUEST: &[u8] = b"M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nMAN: \"ssdp:discover\"\r\nMX: 1\r\nST: ssdp:all\r\nUSER-AGENT: Lantern/0.1 UPnP/1.1\r\n\r\n";
    let socket = match tokio::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await {
        Ok(socket) => socket,
        Err(error) => {
            let _ = sender
                .send(DiscoveryEvent::ProviderHealth(ProviderHealth {
                    provider: "ssdp",
                    status: HealthStatus::Degraded,
                    detail: format!("could not bind UDP socket: {error}"),
                }))
                .await;
            return;
        }
    };
    if let Err(error) = socket
        .send_to(REQUEST, (Ipv4Addr::new(239, 255, 255, 250), 1900))
        .await
    {
        let _ = sender
            .send(DiscoveryEvent::ProviderHealth(ProviderHealth {
                provider: "ssdp",
                status: HealthStatus::Degraded,
                detail: format!("multicast request failed: {error}"),
            }))
            .await;
        return;
    }
    let deadline = tokio::time::Instant::now() + Duration::from_millis(1_100);
    let mut buffer = [0_u8; 4_096];
    loop {
        let received = tokio::select! {
            () = cancellation.cancelled() => return,
            result = tokio::time::timeout_at(deadline, socket.recv_from(&mut buffer)) => result,
        };
        let Ok(Ok((length, source))) = received else {
            break;
        };
        let IpAddr::V4(ip) = source.ip() else {
            continue;
        };
        if !spec.subnet.contains(&ip) {
            continue;
        }
        found_ips.insert(ip);
        let headers = String::from_utf8_lossy(&buffer[..length]);
        let server_name = headers.lines().find_map(|line| {
            line.split_once(':').and_then(|(name, value)| {
                name.eq_ignore_ascii_case("server")
                    .then(|| value.trim().to_string())
            })
        });
        let observation = Observation {
            ip,
            mac: None,
            hostname: server_name,
            vendor: None,
            open_port: None,
            latency: None,
            source: DiscoverySource::Ssdp,
            interface: spec.interface.name.clone(),
            subnet: spec.subnet.to_string(),
            observed_at: Utc::now(),
            http: None,
        };
        if sender
            .send(DiscoveryEvent::Observation(observation))
            .await
            .is_err()
        {
            break;
        }
    }
}

async fn safe_http_metadata(
    stream: &mut TcpStream,
    ip: Ipv4Addr,
    timeout: Duration,
    cancellation: &CancellationToken,
) -> Option<HttpMetadata> {
    let request = format!(
        "GET / HTTP/1.0\r\nHost: {ip}\r\nUser-Agent: Lantern/{}\r\nAccept: text/html,*/*;q=0.1\r\nConnection: close\r\n\r\n",
        crate::VERSION
    );
    let mut bytes = Vec::with_capacity(8_192);
    let exchange = async {
        stream.write_all(request.as_bytes()).await.ok()?;
        stream.flush().await.ok()?;
        stream.take(16_384).read_to_end(&mut bytes).await.ok()?;
        Some(())
    };
    tokio::select! {
        () = cancellation.cancelled() => return None,
        result = tokio::time::timeout(timeout.saturating_mul(3), exchange) => {
            result.ok().flatten()?;
        }
    }
    parse_http_metadata(&bytes)
}

fn parse_http_metadata(bytes: &[u8]) -> Option<HttpMetadata> {
    let response = String::from_utf8_lossy(bytes);
    let (headers, body) = response
        .split_once("\r\n\r\n")
        .or_else(|| response.split_once("\n\n"))
        .unwrap_or((response.as_ref(), ""));
    let mut lines = headers.lines();
    let status = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse::<u16>().ok());
    let mut server = None;
    let mut redirect = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.eq_ignore_ascii_case("server") {
            server = Some(value.trim().chars().take(256).collect());
        } else if name.eq_ignore_ascii_case("location") {
            redirect = Some(value.trim().chars().take(1_024).collect());
        }
    }
    let body_lower = body.to_ascii_lowercase();
    let title = body_lower.find("<title").and_then(|start| {
        let content_start = body[start..].find('>')? + start + 1;
        let end = body_lower[content_start..].find("</title>")? + content_start;
        Some(
            body[content_start..end]
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(256)
                .collect(),
        )
    });
    (status.is_some() || server.is_some() || redirect.is_some() || title.is_some()).then_some(
        HttpMetadata {
            status,
            title,
            server,
            redirect,
        },
    )
}

const fn netmask_prefix(netmask: Ipv4Addr) -> u8 {
    netmask.to_bits().count_ones() as u8
}

fn network_address(address: Ipv4Addr, prefix: u8) -> Ipv4Addr {
    let mask = if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix)
    };
    Ipv4Addr::from(u32::from(address) & mask)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capability_status(config: &Config, provider: &str) -> HealthStatus {
        provider_capabilities(config)
            .into_iter()
            .find(|health| health.provider == provider)
            .expect("provider capability")
            .status
    }

    #[test]
    fn prefix_is_calculated_from_netmask() {
        assert_eq!(netmask_prefix(Ipv4Addr::new(255, 255, 255, 0)), 24);
        assert_eq!(netmask_prefix(Ipv4Addr::new(255, 255, 0, 0)), 16);
        assert_eq!(
            network_address(Ipv4Addr::new(172, 18, 4, 7), 16),
            Ipv4Addr::new(172, 18, 0, 0)
        );
    }

    #[test]
    fn parses_linux_default_route_and_prefers_lowest_metric() {
        let routes = concat!(
            "Iface Destination Gateway Flags RefCnt Use Metric Mask MTU Window IRTT\n",
            "eth0 00000000 0100000A 0003 0 0 100 00000000 0 0 0\n",
            "eth1 00000000 0128A8C0 0003 0 0 50 00000000 0 0 0\n",
            "bad0 00000000 0100007F 0001 0 0 1 00000000 0 0 0\n",
        );
        let route = parse_default_route(routes).expect("default route");
        assert_eq!(route.interface, "eth1");
        assert_eq!(route.gateway, Ipv4Addr::new(192, 168, 40, 1));
    }

    #[test]
    fn default_optional_providers_are_disabled_not_broken() {
        let config = Config::default();
        for provider in ["icmp", "netbios", "http-metadata", "tls-metadata"] {
            assert_eq!(capability_status(&config, provider), HealthStatus::Disabled);
        }
    }

    #[test]
    fn requested_unsupported_providers_are_unavailable() {
        let mut config = Config::default();
        config.enabled_protocols.icmp = true;
        config.enabled_protocols.netbios = true;
        config.tls_metadata = true;
        for provider in ["icmp", "netbios", "tls-metadata"] {
            assert_eq!(
                capability_status(&config, provider),
                HealthStatus::Unavailable
            );
        }
    }

    #[test]
    fn passive_mode_marks_tcp_dependent_capabilities_inactive() {
        let config = Config {
            scan_mode: ScanMode::Passive,
            http_metadata: true,
            ..Config::default()
        };
        assert_eq!(capability_status(&config, "tcp"), HealthStatus::Disabled);
        assert_eq!(
            capability_status(&config, "http-metadata"),
            HealthStatus::Disabled
        );
    }

    #[test]
    fn scan_rejects_networks_over_safety_limit() {
        let config = Config {
            subnet: Some("10.0.0.0/8".into()),
            host_limit: 256,
            ..Config::default()
        };
        let interface = NetworkInterface {
            name: "test0".into(),
            address: Ipv4Addr::new(10, 0, 0, 2),
            netmask: Ipv4Addr::new(255, 0, 0, 0),
            subnet: "10.0.0.0/8".parse().expect("subnet"),
            is_loopback: false,
            likely_active: true,
        };
        assert!(ScanSpec::from_config(&config, interface).is_err());
    }

    #[test]
    fn scan_spec_respects_disabled_local_and_tcp_providers() {
        let mut config = Config {
            subnet: Some("192.0.2.0/30".into()),
            scan_mode: ScanMode::Normal,
            ..Config::default()
        };
        config.enabled_protocols.local = false;
        config.enabled_protocols.tcp = false;
        let interface = NetworkInterface {
            name: "mock0".into(),
            address: Ipv4Addr::new(192, 0, 2, 1),
            netmask: Ipv4Addr::new(255, 255, 255, 252),
            subnet: "192.0.2.0/30".parse().expect("subnet"),
            is_loopback: false,
            likely_active: true,
        };
        let spec = ScanSpec::from_config(&config, interface).expect("scan spec");
        assert!(!spec.local);
        assert!(spec.ports.is_empty());
        assert!(spec.arp);
        assert!(spec.mdns);
        assert!(spec.ssdp);
    }

    #[test]
    fn parses_bounded_http_metadata_without_following_redirects() {
        let response = b"HTTP/1.1 302 Found\r\nServer: tiny-device\r\nLocation: /login\r\n\r\n<html><title> Device Console </title></html>";
        let metadata = parse_http_metadata(response).expect("metadata");
        assert_eq!(metadata.status, Some(302));
        assert_eq!(metadata.server.as_deref(), Some("tiny-device"));
        assert_eq!(metadata.redirect.as_deref(), Some("/login"));
        assert_eq!(metadata.title.as_deref(), Some("Device Console"));
    }

    #[tokio::test]
    async fn mock_provider_streams_incremental_events_without_network_access() {
        let config = Config {
            subnet: Some("192.0.2.0/30".into()),
            ..Config::default()
        };
        let interface = NetworkInterface {
            name: "mock0".into(),
            address: Ipv4Addr::new(192, 0, 2, 1),
            netmask: Ipv4Addr::new(255, 255, 255, 252),
            subnet: "192.0.2.0/30".parse().expect("subnet"),
            is_loopback: false,
            likely_active: true,
        };
        let spec = ScanSpec::from_config(&config, interface).expect("spec");
        let observation = Observation::tcp(
            Ipv4Addr::new(192, 0, 2, 2),
            443,
            Duration::from_millis(2),
            "mock0",
            "192.0.2.0/30",
        );
        let (sender, mut receiver) = mpsc::channel(8);
        let task = spawn_mock_scan(spec, vec![observation], sender, CancellationToken::new());
        let mut started = false;
        let mut observed = false;
        let mut finished = false;
        while let Some(event) = receiver.recv().await {
            match event {
                DiscoveryEvent::Started { .. } => started = true,
                DiscoveryEvent::Observation(_) => observed = true,
                DiscoveryEvent::Finished { .. } => {
                    finished = true;
                    break;
                }
                DiscoveryEvent::Progress(_)
                | DiscoveryEvent::ProviderHealth(_)
                | DiscoveryEvent::Failed { .. } => {}
            }
        }
        task.await.expect("mock worker");
        assert!(started && observed && finished);
    }
}
