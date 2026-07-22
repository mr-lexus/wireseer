use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    net::{IpAddr, Ipv4Addr},
    time::Duration,
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceStatus {
    Online,
    Offline,
    #[default]
    Unknown,
    Stale,
    Scanning,
}

impl fmt::Display for DeviceStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Online => "online",
            Self::Offline => "offline",
            Self::Unknown => "unknown",
            Self::Stale => "stale",
            Self::Scanning => "scanning",
        })
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceType {
    Router,
    Gateway,
    Computer,
    Phone,
    Tablet,
    SmartTv,
    Printer,
    Nas,
    Server,
    Camera,
    GameConsole,
    SmartHome,
    VirtualMachine,
    ContainerHost,
    #[default]
    Unknown,
}

impl DeviceType {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Router => "Router",
            Self::Gateway => "Gateway",
            Self::Computer => "Computer",
            Self::Phone => "Phone",
            Self::Tablet => "Tablet",
            Self::SmartTv => "Smart TV",
            Self::Printer => "Printer",
            Self::Nas => "NAS",
            Self::Server => "Server",
            Self::Camera => "Camera",
            Self::GameConsole => "Console",
            Self::SmartHome => "Smart home",
            Self::VirtualMachine => "Virtual machine",
            Self::ContainerHost => "Container host",
            Self::Unknown => "Unknown",
        }
    }

    pub const fn ascii_icon(self) -> &'static str {
        match self {
            Self::Router | Self::Gateway => "[R]",
            Self::Computer | Self::VirtualMachine | Self::ContainerHost => "[PC]",
            Self::Phone | Self::Tablet => "[M]",
            Self::SmartTv => "[TV]",
            Self::Printer => "[P]",
            Self::Nas | Self::Server => "[NAS]",
            Self::Camera => "[C]",
            Self::GameConsole => "[G]",
            Self::SmartHome => "[IoT]",
            Self::Unknown => "[?]",
        }
    }
}

impl fmt::Display for DeviceType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum DiscoverySource {
    Local,
    Arp,
    Icmp,
    Tcp,
    ReverseDns,
    Mdns,
    Ssdp,
    NetBios,
    History,
    User,
}

impl fmt::Display for DiscoverySource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Local => "local",
            Self::Arp => "ARP",
            Self::Icmp => "ICMP",
            Self::Tcp => "TCP",
            Self::ReverseDns => "DNS",
            Self::Mdns => "mDNS",
            Self::Ssdp => "SSDP",
            Self::NetBios => "NetBIOS",
            Self::History => "history",
            Self::User => "user",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    pub source: DiscoverySource,
    pub description: String,
    pub weight: i16,
    pub observed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Service {
    pub port: u16,
    pub transport: Transport,
    pub name: String,
    pub state: ServiceState,
    pub latency_ms: Option<u32>,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
    pub sources: BTreeSet<DiscoverySource>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    Tcp,
    Udp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceState {
    Open,
    Advertised,
    Historical,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpMetadata {
    pub status: Option<u16>,
    pub title: Option<String>,
    pub server: Option<String>,
    pub redirect: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TlsMetadata {
    pub subject: Option<String>,
    pub issuer: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UserMetadata {
    pub name: Option<String>,
    pub tags: BTreeSet<String>,
    pub notes: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    pub id: Uuid,
    pub stable_key: String,
    pub ipv4: Ipv4Addr,
    pub ipv6: Vec<IpAddr>,
    pub previous_addresses: Vec<IpAddr>,
    pub mac: Option<String>,
    pub vendor: Option<String>,
    pub hostname: Option<String>,
    pub device_type: DeviceType,
    #[serde(default)]
    pub inferred_model: Option<String>,
    #[serde(default)]
    pub platform: Option<String>,
    pub status: DeviceStatus,
    pub latency_ms: Option<u32>,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
    pub last_changed: DateTime<Utc>,
    pub interface: String,
    pub subnet: String,
    pub services: BTreeMap<u16, Service>,
    pub http: BTreeMap<u16, HttpMetadata>,
    pub tls: BTreeMap<u16, TlsMetadata>,
    pub sources: BTreeSet<DiscoverySource>,
    pub confidence: u8,
    pub evidence: Vec<Evidence>,
    pub user: UserMetadata,
    pub changed: bool,
    pub baseline_unknown: bool,
}

impl Device {
    #[must_use]
    pub fn from_observation(observation: &Observation) -> Self {
        let now = observation.observed_at;
        let mut sources = BTreeSet::new();
        sources.insert(observation.source);
        let mut device = Self {
            id: Uuid::new_v4(),
            stable_key: observation
                .mac
                .clone()
                .unwrap_or_else(|| observation.ip.to_string()),
            ipv4: observation.ip,
            ipv6: Vec::new(),
            previous_addresses: Vec::new(),
            mac: observation.mac.clone(),
            vendor: observation.vendor.clone(),
            hostname: observation.hostname.clone(),
            device_type: DeviceType::Unknown,
            inferred_model: None,
            platform: None,
            status: DeviceStatus::Online,
            latency_ms: observation.latency.map(duration_ms),
            first_seen: now,
            last_seen: now,
            last_changed: now,
            interface: observation.interface.clone(),
            subnet: observation.subnet.clone(),
            services: BTreeMap::new(),
            http: BTreeMap::new(),
            tls: BTreeMap::new(),
            sources,
            confidence: 0,
            evidence: Vec::new(),
            user: UserMetadata::default(),
            changed: true,
            baseline_unknown: false,
        };
        if let Some(port) = observation.open_port {
            device.upsert_service(port, observation.source, now, observation.latency);
            if let Some(http) = &observation.http {
                device.http.insert(port, http.clone());
            }
        }
        device.recalculate_fingerprint();
        device
    }

    pub fn merge(&mut self, observation: &Observation) -> Vec<DeviceChange> {
        let mut changes = Vec::new();
        self.last_seen = self.last_seen.max(observation.observed_at);
        self.status = DeviceStatus::Online;
        self.sources.insert(observation.source);
        if let Some(latency) = observation.latency {
            self.latency_ms = Some(duration_ms(latency));
        }
        if let Some(observed_mac) = &observation.mac {
            match &self.mac {
                None => {
                    self.mac = Some(observed_mac.clone());
                    self.stable_key.clone_from(observed_mac);
                    changes.push(DeviceChange::Identity("MAC address observed".into()));
                }
                Some(current) if current != observed_mac => {
                    let detail = format!("MAC address changed: {current} -> {observed_mac}");
                    self.mac = Some(observed_mac.clone());
                    self.stable_key.clone_from(observed_mac);
                    changes.push(DeviceChange::Identity(detail));
                }
                Some(_) => {}
            }
        }
        if self.hostname.is_none() && observation.hostname.is_some() {
            self.hostname.clone_from(&observation.hostname);
            changes.push(DeviceChange::Identity("hostname observed".into()));
        }
        if self.vendor.is_none() && observation.vendor.is_some() {
            self.vendor.clone_from(&observation.vendor);
            changes.push(DeviceChange::Identity("vendor observed".into()));
        }
        if let Some(port) = observation.open_port {
            let is_new = !self.services.contains_key(&port);
            self.upsert_service(
                port,
                observation.source,
                observation.observed_at,
                observation.latency,
            );
            if is_new {
                changes.push(DeviceChange::ServiceAdded(port));
            }
            if let Some(http) = &observation.http {
                self.http.insert(port, http.clone());
            }
        }
        let old_type = self.device_type;
        let old_confidence = self.confidence;
        self.recalculate_fingerprint();
        if self.device_type != old_type || self.confidence != old_confidence {
            changes.push(DeviceChange::Fingerprint {
                from: old_type,
                to: self.device_type,
            });
        }
        if !changes.is_empty() {
            self.changed = true;
            self.last_changed = observation.observed_at;
        }
        changes
    }

    fn upsert_service(
        &mut self,
        port: u16,
        source: DiscoverySource,
        seen: DateTime<Utc>,
        latency: Option<Duration>,
    ) {
        let service = self.services.entry(port).or_insert_with(|| Service {
            port,
            transport: Transport::Tcp,
            name: service_name(port).to_string(),
            state: ServiceState::Open,
            latency_ms: latency.map(duration_ms),
            first_seen: seen,
            last_seen: seen,
            sources: BTreeSet::new(),
        });
        service.last_seen = seen;
        service.sources.insert(source);
    }

    pub fn recalculate_fingerprint(&mut self) {
        let fingerprint = fingerprint(
            self.vendor.as_deref(),
            self.hostname.as_deref(),
            &self.services,
            &self.http,
            &self.sources,
            self.ipv4,
        );
        let score: i16 = fingerprint.evidence.iter().map(|item| item.weight).sum();
        self.device_type = fingerprint.kind;
        self.inferred_model = fingerprint.model;
        self.platform = fingerprint.platform;
        self.confidence = score.clamp(0, 100) as u8;
        self.evidence = fingerprint.evidence;
    }

    #[must_use]
    pub fn display_name(&self) -> String {
        self.user
            .name
            .clone()
            .or_else(|| self.inferred_model.clone())
            .or_else(|| self.hostname.clone())
            .or_else(|| {
                self.vendor
                    .as_ref()
                    .map(|vendor| format!("{vendor} device"))
            })
            .unwrap_or_else(|| {
                self.mac.as_deref().map_or_else(
                    || "Unknown device".into(),
                    |mac| format!("Unknown · {}", mac_prefix(mac)),
                )
            })
    }

    /// Returns a concise presentation name while preserving the complete IEEE
    /// registry value in serialized exports.
    #[must_use]
    pub fn manufacturer_name(&self) -> Option<String> {
        self.vendor.as_deref().map(|vendor| {
            let lower = vendor.to_ascii_lowercase();
            if lower.contains("midea") {
                "Midea".into()
            } else if lower.contains("heights telecom") {
                "Heights Telecom".into()
            } else if lower.contains("liteon") || lower.contains("lite-on") {
                "Lite-On".into()
            } else {
                vendor.to_string()
            }
        })
    }

    #[must_use]
    pub fn uses_private_mac(&self) -> bool {
        self.mac
            .as_deref()
            .and_then(|mac| mac.split([':', '-']).next())
            .and_then(|octet| u8::from_str_radix(octet, 16).ok())
            .is_some_and(|octet| octet & 0x02 != 0)
    }

    #[must_use]
    pub fn identity_is_user_confirmed(&self) -> bool {
        self.user.name.is_some()
    }
}

fn mac_prefix(mac: &str) -> String {
    mac.split([':', '-']).take(3).collect::<Vec<_>>().join(":")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeviceChange {
    Identity(String),
    ServiceAdded(u16),
    ServiceRemoved(u16),
    AddressChanged {
        from: IpAddr,
        to: IpAddr,
    },
    Fingerprint {
        from: DeviceType,
        to: DeviceType,
    },
    Status {
        from: DeviceStatus,
        to: DeviceStatus,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observation {
    pub ip: Ipv4Addr,
    pub mac: Option<String>,
    pub hostname: Option<String>,
    pub vendor: Option<String>,
    pub open_port: Option<u16>,
    pub latency: Option<Duration>,
    pub source: DiscoverySource,
    pub interface: String,
    pub subnet: String,
    pub observed_at: DateTime<Utc>,
    pub http: Option<HttpMetadata>,
}

impl Observation {
    #[must_use]
    pub fn tcp(ip: Ipv4Addr, port: u16, latency: Duration, interface: &str, subnet: &str) -> Self {
        Self {
            ip,
            mac: None,
            hostname: None,
            vendor: None,
            open_port: Some(port),
            latency: Some(latency),
            source: DiscoverySource::Tcp,
            interface: interface.into(),
            subnet: subnet.into(),
            observed_at: Utc::now(),
            http: None,
        }
    }
}

#[must_use]
pub fn service_name(port: u16) -> &'static str {
    match port {
        22 => "SSH",
        23 => "Telnet",
        53 => "DNS",
        80 | 8080 | 8008 => "HTTP",
        443 | 8443 => "HTTPS",
        139 | 445 => "SMB",
        548 => "AFP",
        631 | 9100 => "Printer",
        1883 | 8883 => "MQTT",
        2049 => "NFS",
        3306 => "MySQL",
        3389 => "RDP",
        5000 | 5001 => "Web UI",
        5432 => "PostgreSQL",
        5900 => "VNC",
        6379 => "Redis",
        8009 => "Media",
        32400 => "Plex",
        _ => "TCP",
    }
}

fn duration_ms(duration: Duration) -> u32 {
    u32::try_from(duration.as_millis()).unwrap_or(u32::MAX)
}

struct Fingerprint {
    kind: DeviceType,
    evidence: Vec<Evidence>,
    model: Option<String>,
    platform: Option<String>,
}

fn fingerprint(
    vendor: Option<&str>,
    hostname: Option<&str>,
    services: &BTreeMap<u16, Service>,
    http: &BTreeMap<u16, HttpMetadata>,
    sources: &BTreeSet<DiscoverySource>,
    ip: Ipv4Addr,
) -> Fingerprint {
    let hostname_lower = hostname.unwrap_or_default().to_ascii_lowercase();
    let normalized_hostname = hostname_lower.replace(['_', ' '], "-");
    let now = Utc::now();
    let vendor_lower = vendor.unwrap_or_default().to_ascii_lowercase();
    let mut candidates: Vec<(DeviceType, Evidence)> = Vec::new();
    let mut add = |kind, source, description: &str, weight| {
        candidates.push((
            kind,
            Evidence {
                source,
                description: description.into(),
                weight,
                observed_at: now,
            },
        ));
    };
    if sources.contains(&DiscoverySource::Local) {
        add(
            DeviceType::Computer,
            DiscoverySource::Local,
            "address belongs to this computer",
            55,
        );
    }
    if ip.octets()[3] == 1 {
        add(
            DeviceType::Gateway,
            DiscoverySource::Local,
            "address is the common subnet gateway",
            60,
        );
    }
    if ip.octets()[3] == 1 && services.contains_key(&53) {
        add(
            DeviceType::Gateway,
            DiscoverySource::Tcp,
            "gateway address also provides DNS",
            20,
        );
    }
    const VENDOR_RULES: &[(DeviceType, &[&str], &str, i16)] = &[
        (
            DeviceType::Nas,
            &["synology", "qnap"],
            "vendor is associated with network storage",
            35,
        ),
        (
            DeviceType::Printer,
            &["brother", "epson", "xerox", "lexmark", "ricoh"],
            "vendor primarily manufactures printers",
            28,
        ),
        (
            DeviceType::Camera,
            &["hikvision", "dahua", "axis communications", "reolink"],
            "vendor is associated with network cameras",
            40,
        ),
        (
            DeviceType::Router,
            &[
                "ubiquiti",
                "mikrotik",
                "tp-link",
                "netgear",
                "zyxel",
                "heights telecom",
            ],
            "vendor is associated with network infrastructure",
            25,
        ),
        (
            DeviceType::SmartHome,
            &["espressif", "tuya", "shelly", "midea"],
            "vendor is associated with smart-home devices",
            30,
        ),
        (
            DeviceType::SmartTv,
            &["roku", "vizio"],
            "vendor is associated with streaming or television devices",
            35,
        ),
        (
            DeviceType::GameConsole,
            &["nintendo"],
            "vendor is associated with game consoles",
            35,
        ),
        (
            DeviceType::VirtualMachine,
            &["vmware"],
            "vendor prefix belongs to a virtual machine platform",
            45,
        ),
        (
            DeviceType::Computer,
            &["raspberry pi", "dell", "lenovo"],
            "vendor is associated with computers",
            20,
        ),
        (
            DeviceType::Computer,
            &["apple"],
            "vendor is Apple (device family remains uncertain)",
            12,
        ),
    ];
    for &(kind, patterns, description, weight) in VENDOR_RULES {
        if patterns
            .iter()
            .any(|pattern| vendor_lower.contains(pattern))
        {
            add(kind, DiscoverySource::Arp, description, weight);
        }
    }

    let specific_hostname = if normalized_hostname.contains("s23-fe") {
        add(
            DeviceType::Phone,
            DiscoverySource::ReverseDns,
            "hostname identifies a Samsung Galaxy S23 FE phone",
            75,
        );
        true
    } else if normalized_hostname.contains("redmi-note-11-pro") {
        add(
            DeviceType::Phone,
            DiscoverySource::ReverseDns,
            "hostname identifies a Xiaomi Redmi Note 11 Pro phone",
            75,
        );
        true
    } else if hostname_lower.contains("iphone") {
        add(
            DeviceType::Phone,
            DiscoverySource::ReverseDns,
            "hostname identifies an Apple phone (iPhone family)",
            65,
        );
        true
    } else if hostname_lower.contains("macbookpro") || normalized_hostname.contains("macbook-pro") {
        add(
            DeviceType::Computer,
            DiscoverySource::ReverseDns,
            "hostname identifies an Apple MacBook Pro computer",
            65,
        );
        true
    } else if normalized_hostname.contains("net-ac-") {
        add(
            DeviceType::SmartHome,
            DiscoverySource::ReverseDns,
            "hostname identifies a Midea air conditioner",
            65,
        );
        true
    } else if normalized_hostname.contains("midea-e1-") {
        add(
            DeviceType::SmartHome,
            DiscoverySource::ReverseDns,
            "hostname identifies a Midea appliance family",
            35,
        );
        true
    } else {
        false
    };

    const HOSTNAME_RULES: &[(DeviceType, &[&str], &str, i16)] = &[
        (
            DeviceType::Phone,
            &["android", "pixel-", "galaxy-", "s24-", "sm-"],
            "hostname resembles a phone",
            45,
        ),
        (
            DeviceType::Tablet,
            &["ipad"],
            "hostname resembles a tablet",
            50,
        ),
        (
            DeviceType::Computer,
            &["imac", "desktop", "laptop", "workstation"],
            "hostname resembles a computer",
            35,
        ),
        (
            DeviceType::Printer,
            &["printer", "epson", "brother"],
            "hostname identifies a printer",
            45,
        ),
        (
            DeviceType::Nas,
            &["diskstation", "qnap", "-nas", "nas-"],
            "hostname identifies network storage",
            45,
        ),
        (
            DeviceType::Camera,
            &["camera", "ipcam", "doorbell"],
            "hostname resembles a network camera",
            40,
        ),
        (
            DeviceType::Router,
            &["router", "gateway", "openwrt", "fritz"],
            "hostname identifies network infrastructure",
            40,
        ),
        (
            DeviceType::SmartTv,
            &["chromecast", "appletv", "firetv", "roku", "bravia"],
            "hostname identifies a television or streaming device",
            45,
        ),
        (
            DeviceType::GameConsole,
            &["xbox", "playstation", "nintendo"],
            "hostname identifies a game console",
            50,
        ),
        (
            DeviceType::SmartHome,
            &["homeassistant", "shelly", "tuya", "tasmota"],
            "hostname identifies a smart-home device",
            40,
        ),
    ];
    if !specific_hostname {
        for &(kind, patterns, description, weight) in HOSTNAME_RULES {
            if patterns
                .iter()
                .any(|pattern| hostname_lower.contains(pattern))
            {
                add(kind, DiscoverySource::ReverseDns, description, weight);
            }
        }
    }
    if services.contains_key(&9100) || services.contains_key(&631) {
        add(
            DeviceType::Printer,
            DiscoverySource::Tcp,
            "printing service is available",
            45,
        );
    }
    if services.contains_key(&5000) && services.contains_key(&5001) {
        add(
            DeviceType::Nas,
            DiscoverySource::Tcp,
            "common NAS management ports are available",
            40,
        );
    }
    if services.contains_key(&445) || services.contains_key(&2049) {
        add(
            DeviceType::Nas,
            DiscoverySource::Tcp,
            "file-sharing service is available",
            25,
        );
    }
    if services.contains_key(&22) && (services.contains_key(&80) || services.contains_key(&443)) {
        add(
            DeviceType::Server,
            DiscoverySource::Tcp,
            "remote shell and web services are available",
            30,
        );
    }
    if services.contains_key(&32400) || services.contains_key(&8009) {
        add(
            DeviceType::SmartTv,
            DiscoverySource::Tcp,
            "media service is available",
            30,
        );
    }
    for metadata in http.values() {
        let title = metadata
            .title
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if title.contains("diskstation") || title.contains("synology") {
            add(
                DeviceType::Nas,
                DiscoverySource::Tcp,
                "HTTP title identifies a NAS management interface",
                30,
            );
        } else if title.contains("printer") {
            add(
                DeviceType::Printer,
                DiscoverySource::Tcp,
                "HTTP title identifies a printer interface",
                28,
            );
        }
    }

    let mut totals: BTreeMap<String, (DeviceType, i16)> = BTreeMap::new();
    for (kind, evidence) in &candidates {
        let entry = totals.entry(kind.label().into()).or_insert((*kind, 0));
        entry.1 += evidence.weight;
    }
    let best = totals.values().max_by_key(|(_, score)| *score).copied();
    if let Some((kind, _)) = best {
        let evidence = candidates
            .into_iter()
            .filter_map(|(candidate, evidence)| (candidate == kind).then_some(evidence))
            .collect();
        let (model, platform) = inferred_identity(kind, &hostname_lower, &vendor_lower);
        Fingerprint {
            kind,
            evidence,
            model,
            platform,
        }
    } else {
        Fingerprint {
            kind: DeviceType::Unknown,
            evidence: Vec::new(),
            model: None,
            platform: None,
        }
    }
}

fn inferred_identity(
    kind: DeviceType,
    hostname: &str,
    vendor: &str,
) -> (Option<String>, Option<String>) {
    let normalized = hostname.replace(['_', ' '], "-");
    let pair = match kind {
        DeviceType::Phone if normalized.contains("s23-fe") => {
            (Some("Samsung Galaxy S23 FE"), Some("Android"))
        }
        DeviceType::Phone if normalized.contains("redmi-note-11-pro") => {
            (Some("Xiaomi Redmi Note 11 Pro"), Some("Android"))
        }
        DeviceType::Phone if hostname.contains("iphone") => (Some("Apple iPhone"), Some("iOS")),
        DeviceType::Computer
            if hostname.contains("macbookpro") || normalized.contains("macbook-pro") =>
        {
            (Some("Apple MacBook Pro"), Some("macOS"))
        }
        DeviceType::Computer if hostname.starts_with("desktop-") => {
            (Some("Windows PC"), Some("Windows"))
        }
        DeviceType::SmartHome if normalized.contains("net-ac-") => {
            (Some("Midea air conditioner"), None)
        }
        DeviceType::SmartHome if normalized.contains("midea-e1-") => {
            (Some("Midea appliance"), None)
        }
        DeviceType::SmartHome if vendor.contains("midea") => (Some("Midea smart appliance"), None),
        DeviceType::Gateway | DeviceType::Router
            if vendor.contains("heights telecom") || hostname.contains("heights") =>
        {
            (Some("Heights gateway"), None)
        }
        DeviceType::Gateway => (Some("Network gateway"), None),
        _ => (None, None),
    };
    (pair.0.map(str::to_string), pair.1.map(str::to_string))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation(ip: Ipv4Addr, port: Option<u16>) -> Observation {
        Observation {
            ip,
            mac: None,
            hostname: None,
            vendor: None,
            open_port: port,
            latency: Some(Duration::from_millis(3)),
            source: DiscoverySource::Tcp,
            interface: "test0".into(),
            subnet: "192.0.2.0/24".into(),
            observed_at: Utc::now(),
            http: None,
        }
    }

    #[test]
    fn merges_services_without_duplicates() {
        let obs = observation(Ipv4Addr::new(192, 0, 2, 4), Some(443));
        let mut device = Device::from_observation(&obs);
        device.merge(&obs);
        assert_eq!(device.services.len(), 1);
        assert!(device.services.contains_key(&443));
    }

    #[test]
    fn fingerprint_is_explainable() {
        let mut device =
            Device::from_observation(&observation(Ipv4Addr::new(192, 0, 2, 5), Some(9100)));
        device.recalculate_fingerprint();
        assert_eq!(device.device_type, DeviceType::Printer);
        assert_eq!(device.confidence, 45);
        assert!(device.evidence[0].description.contains("printing"));
    }

    #[test]
    fn hostname_identifies_common_device_families() {
        let mut observation = observation(Ipv4Addr::new(192, 0, 2, 6), None);
        observation.hostname = Some("living-room-chromecast".into());
        let device = Device::from_observation(&observation);
        assert_eq!(device.device_type, DeviceType::SmartTv);
        assert_eq!(device.confidence, 45);
        assert_eq!(device.evidence[0].source, DiscoverySource::ReverseDns);

        observation.hostname = Some("S23-FE-user".into());
        assert_eq!(
            Device::from_observation(&observation).device_type,
            DeviceType::Phone
        );

        observation.hostname = Some("net_ac_4E46.local".into());
        assert_eq!(
            Device::from_observation(&observation).device_type,
            DeviceType::SmartHome
        );

        observation.hostname = None;
        observation.vendor = Some("GD Midea Air-Conditioning Equipment Co.,Ltd.".into());
        assert_eq!(
            Device::from_observation(&observation).device_type,
            DeviceType::SmartHome
        );

        observation.hostname = Some("This computer".into());
        observation.vendor = None;
        observation.source = DiscoverySource::Local;
        assert_eq!(
            Device::from_observation(&observation).device_type,
            DeviceType::Computer
        );
    }

    #[test]
    fn hostname_outweighs_an_ambiguous_vendor_family() {
        let mut observation = observation(Ipv4Addr::new(192, 0, 2, 7), None);
        observation.hostname = Some("Alex-iPhone".into());
        observation.vendor = Some("Apple, Inc.".into());
        let device = Device::from_observation(&observation);
        assert_eq!(device.device_type, DeviceType::Phone);
        assert!(device.evidence[0].description.contains("phone"));
    }

    #[test]
    fn a_gateway_address_is_only_an_inference() {
        let device =
            Device::from_observation(&observation(Ipv4Addr::new(192, 0, 2, 1), Some(9100)));
        assert_eq!(device.device_type, DeviceType::Gateway);
        assert!(device.confidence < 100);
    }

    #[test]
    fn normalizes_well_known_services() {
        assert_eq!(service_name(22), "SSH");
        assert_eq!(service_name(445), "SMB");
        assert_eq!(service_name(9100), "Printer");
        assert_eq!(service_name(5000), "Web UI");
        assert_eq!(service_name(49_999), "TCP");
    }

    #[test]
    fn requested_hostnames_resolve_to_honest_device_families() {
        let cases = [
            (
                "MacBookPro.local",
                DeviceType::Computer,
                "Apple MacBook Pro",
                "macOS",
            ),
            (
                "S23-FE-pol-zovatela-Olga.local",
                DeviceType::Phone,
                "Samsung Galaxy S23 FE",
                "Android",
            ),
            ("iPhone.local", DeviceType::Phone, "Apple iPhone", "iOS"),
            (
                "Redmi-Note-11-Pro.local",
                DeviceType::Phone,
                "Xiaomi Redmi Note 11 Pro",
                "Android",
            ),
        ];

        for (hostname, kind, model, platform) in cases {
            let mut observed = observation(Ipv4Addr::new(192, 0, 2, 20), None);
            observed.hostname = Some(hostname.into());
            let device = Device::from_observation(&observed);
            assert_eq!(device.device_type, kind, "{hostname}");
            assert_eq!(device.inferred_model.as_deref(), Some(model), "{hostname}");
            assert_eq!(device.platform.as_deref(), Some(platform), "{hostname}");
            assert_eq!(device.display_name(), model, "{hostname}");
            assert!(device.confidence >= 65, "{hostname}");
        }
    }

    #[test]
    fn midea_hostname_distinguishes_air_conditioner_from_unknown_appliance() {
        let mut observed = observation(Ipv4Addr::new(192, 0, 2, 21), None);
        observed.vendor = Some("GD Midea Air-Conditioning Equipment Co.,Ltd.".into());
        observed.hostname = Some("net_ac_91BA.local".into());
        let air_conditioner = Device::from_observation(&observed);
        assert_eq!(
            air_conditioner.inferred_model.as_deref(),
            Some("Midea air conditioner")
        );
        assert_eq!(
            air_conditioner.manufacturer_name().as_deref(),
            Some("Midea")
        );

        observed.hostname = Some("midea_e1_0082.local".into());
        let mut appliance = Device::from_observation(&observed);
        assert_eq!(appliance.device_type, DeviceType::SmartHome);
        assert_eq!(appliance.inferred_model.as_deref(), Some("Midea appliance"));
        assert!(!appliance.display_name().contains("air conditioner"));

        appliance.user.name = Some("Midea dishwasher".into());
        assert_eq!(appliance.display_name(), "Midea dishwasher");
        assert!(appliance.identity_is_user_confirmed());
    }

    #[test]
    fn heights_gateway_uses_network_evidence_and_vendor_identity() {
        let mut observed = observation(Ipv4Addr::new(192, 168, 40, 1), Some(53));
        observed.hostname = Some("Heights".into());
        observed.vendor = Some("Heights Telecom T ltd".into());
        let device = Device::from_observation(&observed);
        assert_eq!(device.device_type, DeviceType::Gateway);
        assert_eq!(device.inferred_model.as_deref(), Some("Heights gateway"));
        assert_eq!(
            device.manufacturer_name().as_deref(),
            Some("Heights Telecom")
        );
        assert!(device.confidence >= 80);
    }

    #[test]
    fn private_mac_is_explained_and_user_can_confirm_exact_model() {
        let mut observed = observation(Ipv4Addr::new(192, 0, 2, 22), None);
        observed.mac = Some("5A:F6:5F:DF:C1:53".into());
        observed.hostname = Some("MacBookPro.local".into());
        let mut device = Device::from_observation(&observed);
        assert!(device.uses_private_mac());
        assert_eq!(device.display_name(), "Apple MacBook Pro");

        device.user.name = Some("MacBook Pro M1".into());
        assert_eq!(device.display_name(), "MacBook Pro M1");
    }

    #[test]
    fn unknown_device_uses_mac_prefix_without_inventing_a_vendor() {
        let mut observed = observation(Ipv4Addr::new(192, 0, 2, 23), None);
        observed.mac = Some("24:54:F2:37:FF:D4".into());
        let device = Device::from_observation(&observed);
        assert_eq!(device.device_type, DeviceType::Unknown);
        assert_eq!(device.inferred_model, None);
        assert_eq!(device.display_name(), "Unknown · 24:54:F2");
        assert!(!device.uses_private_mac());
    }

    #[test]
    fn observed_mac_change_is_explicit() {
        let mut first = observation(Ipv4Addr::new(192, 0, 2, 4), Some(443));
        first.mac = Some("00:11:22:33:44:55".into());
        let mut device = Device::from_observation(&first);
        let mut replacement = first.clone();
        replacement.mac = Some("00:11:22:33:44:66".into());
        let changes = device.merge(&replacement);
        assert!(matches!(
            changes.first(),
            Some(DeviceChange::Identity(detail)) if detail.contains("MAC address changed")
        ));
        assert_eq!(device.mac.as_deref(), Some("00:11:22:33:44:66"));
    }
}
