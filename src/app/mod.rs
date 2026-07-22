use std::{collections::VecDeque, net::Ipv4Addr};

use chrono::{DateTime, Utc};
use fuzzy_matcher::{FuzzyMatcher, skim::SkimMatcherV2};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    alerts::{Alert, Severity},
    config::{Config, IconMode, ScanMode, ThemeName},
    devices::{Device, DeviceChange, DeviceStatus, DeviceType, Observation},
    history::{
        BaselineDiff, DeviceSnapshot, EventKind, ScanDiff, TimelineEvent, diff_snapshots,
        mark_unseen_offline,
    },
    network::{
        DiscoveryEvent, NetworkInterface, ProviderHealth, ScanPhase, ScanProgress, ScanSpec,
        select_interface, spawn_scan, vendor::VendorDatabase,
    },
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Screen {
    #[default]
    Dashboard,
    Devices,
    DeviceDetails,
    History,
    Compare,
    Alerts,
    Logs,
    Settings,
}

impl Screen {
    pub const ALL: [Self; 8] = [
        Self::Dashboard,
        Self::Devices,
        Self::History,
        Self::Compare,
        Self::Alerts,
        Self::Logs,
        Self::Settings,
        Self::DeviceDetails,
    ];

    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Dashboard => "Dashboard",
            Self::Devices => "Devices",
            Self::DeviceDetails => "Device details",
            Self::History => "History",
            Self::Compare => "Compare",
            Self::Alerts => "Alerts",
            Self::Logs => "Logs",
            Self::Settings => "Settings",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Overlay {
    #[default]
    None,
    Help,
    CommandPalette,
    Search,
    Filter,
    Sort,
    Interface,
    ScanMode,
    Theme,
    Export,
    ConfirmQuit,
    Error,
    EditName,
    EditTags,
    EditNotes,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DeviceFilter {
    #[default]
    All,
    Online,
    Offline,
    Unknown,
    Changed,
}

impl DeviceFilter {
    pub const ALL: [Self; 5] = [
        Self::All,
        Self::Online,
        Self::Offline,
        Self::Unknown,
        Self::Changed,
    ];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Online => "online",
            Self::Offline => "offline",
            Self::Unknown => "unknown",
            Self::Changed => "changed",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DeviceSort {
    #[default]
    Status,
    Name,
    Address,
    Vendor,
    Latency,
    LastSeen,
    Confidence,
}

impl DeviceSort {
    pub const ALL: [Self; 7] = [
        Self::Status,
        Self::Name,
        Self::Address,
        Self::Vendor,
        Self::Latency,
        Self::LastSeen,
        Self::Confidence,
    ];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Name => "name",
            Self::Address => "address",
            Self::Vendor => "vendor",
            Self::Latency => "latency",
            Self::LastSeen => "last seen",
            Self::Confidence => "confidence",
        }
    }
}

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub at: DateTime<Utc>,
    pub level: LogLevel,
    pub module: String,
    pub message: String,
    pub fields: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Trace => "TRACE",
            Self::Debug => "DEBUG",
            Self::Info => "INFO ",
            Self::Warn => "WARN ",
            Self::Error => "ERROR",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ScanState {
    pub active: bool,
    pub id: Option<Uuid>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub progress: Option<ScanProgress>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Toast {
    pub message: String,
    pub severity: Severity,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug)]
pub struct AppState {
    pub screen: Screen,
    pub previous_screen: Screen,
    pub overlay: Overlay,
    pub should_quit: bool,
    pub devices: Vec<Device>,
    pub selected: usize,
    pub selected_ids: Vec<Uuid>,
    pub search: String,
    pub palette_query: String,
    pub palette_selected: usize,
    pub filter: DeviceFilter,
    pub settings_selected: usize,
    pub sort: DeviceSort,
    pub compact_rows: bool,
    pub events: VecDeque<TimelineEvent>,
    pub alerts: Vec<Alert>,
    pub logs: VecDeque<LogEntry>,
    pub scan: ScanState,
    pub provider_health: Vec<ProviderHealth>,
    vendor_database: Option<VendorDatabase>,
    pub interfaces: Vec<NetworkInterface>,
    pub active_interface: Option<NetworkInterface>,
    pub scan_mode: ScanMode,
    pub theme: ThemeName,
    pub theme_before_preview: Option<ThemeName>,
    pub icons: IconMode,
    pub animations: bool,
    pub mouse: bool,
    pub visible_columns: Vec<String>,
    pub tick: u64,
    pub toast: Option<Toast>,
    pub error: Option<String>,
    pub editor: String,
    pub pending_persistence: Vec<PersistenceEvent>,
    pub scan_diff: ScanDiff,
    pub baseline_diff: BaselineDiff,
    pub baseline_active: bool,
    pub activity: VecDeque<u64>,
    previous_snapshot: Vec<DeviceSnapshot>,
    scan_failures: u8,
}

impl AppState {
    #[must_use]
    pub fn new(config: &Config, interfaces: Vec<NetworkInterface>, devices: Vec<Device>) -> Self {
        let active_interface = select_interface(&interfaces, config.interface.as_deref()).ok();
        let mut state = Self {
            screen: Screen::Dashboard,
            previous_screen: Screen::Dashboard,
            overlay: Overlay::None,
            should_quit: false,
            devices,
            selected: 0,
            selected_ids: Vec::new(),
            search: String::new(),
            palette_query: String::new(),
            palette_selected: 0,
            settings_selected: 0,
            filter: DeviceFilter::All,
            sort: DeviceSort::Status,
            compact_rows: config.compact_rows,
            events: VecDeque::new(),
            alerts: Vec::new(),
            logs: VecDeque::new(),
            scan: ScanState::default(),
            provider_health: Vec::new(),
            vendor_database: None,
            interfaces,
            active_interface,
            scan_mode: config.scan_mode,
            theme: config.theme,
            theme_before_preview: None,
            icons: config.icons,
            animations: config.animations,
            mouse: config.mouse,
            visible_columns: config.visible_columns.clone(),
            tick: 0,
            toast: None,
            error: None,
            editor: String::new(),
            pending_persistence: Vec::new(),
            scan_diff: ScanDiff::default(),
            baseline_diff: BaselineDiff::default(),
            baseline_active: false,
            activity: VecDeque::from(vec![0; 24]),
            previous_snapshot: Vec::new(),
            scan_failures: 0,
        };
        for device in &mut state.devices {
            device.recalculate_fingerprint();
        }
        state.sort_devices();
        state
    }

    pub fn set_vendor_database(&mut self, database: VendorDatabase) -> usize {
        let mut changed_devices = Vec::new();
        let mut enriched = 0;
        for device in &mut self.devices {
            let old_type = device.device_type;
            let old_confidence = device.confidence;
            let mut changed = false;
            if let Some(vendor) = device.mac.as_deref().and_then(|mac| database.lookup(mac))
                && device.vendor.as_deref() != Some(vendor)
            {
                device.vendor = Some(vendor.to_string());
                enriched += 1;
                changed = true;
            }
            device.recalculate_fingerprint();
            if device.device_type != old_type || device.confidence != old_confidence {
                changed = true;
            }
            if changed {
                changed_devices.push(device.clone());
            }
        }
        self.pending_persistence
            .extend(changed_devices.into_iter().map(PersistenceEvent::Device));
        self.vendor_database = Some(database);
        enriched
    }

    #[must_use]
    pub fn filtered_indices(&self) -> Vec<usize> {
        let matcher = SkimMatcherV2::default();
        let query = self.search.trim();
        self.devices
            .iter()
            .enumerate()
            .filter(|(_, device)| match self.filter {
                DeviceFilter::All => true,
                DeviceFilter::Online => device.status == DeviceStatus::Online,
                DeviceFilter::Offline => device.status == DeviceStatus::Offline,
                DeviceFilter::Unknown => device.device_type == DeviceType::Unknown,
                DeviceFilter::Changed => device.changed,
            })
            .filter_map(|(index, device)| {
                if query.is_empty() {
                    return Some((index, 0));
                }
                let haystack = format!(
                    "{} {} {} {} {} {} {} {} {}",
                    device.display_name(),
                    device.hostname.as_deref().unwrap_or_default(),
                    device.ipv4,
                    device.mac.as_deref().unwrap_or_default(),
                    device.vendor.as_deref().unwrap_or_default(),
                    device
                        .services
                        .values()
                        .map(|item| format!("{} {}", item.port, item.name))
                        .collect::<Vec<_>>()
                        .join(" "),
                    device.device_type,
                    device
                        .user
                        .tags
                        .iter()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(" "),
                    device.user.notes,
                );
                matcher
                    .fuzzy_match(&haystack, query)
                    .map(|score| (index, score))
            })
            .map(|(index, _)| index)
            .collect()
    }

    #[must_use]
    pub fn selected_device(&self) -> Option<&Device> {
        let indices = self.filtered_indices();
        indices
            .get(self.selected)
            .and_then(|index| self.devices.get(*index))
    }

    pub fn next(&mut self) {
        let length = self.filtered_indices().len();
        if length > 0 {
            self.selected = (self.selected + 1).min(length - 1);
        }
    }

    pub fn previous(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn first(&mut self) {
        self.selected = 0;
    }

    pub fn last(&mut self) {
        self.selected = self.filtered_indices().len().saturating_sub(1);
    }

    pub fn toggle_selected(&mut self) {
        let Some(id) = self.selected_device().map(|device| device.id) else {
            return;
        };
        if let Some(position) = self
            .selected_ids
            .iter()
            .position(|selected| *selected == id)
        {
            self.selected_ids.remove(position);
        } else {
            self.selected_ids.push(id);
        }
    }

    pub fn change_screen(&mut self, screen: Screen) {
        if screen != self.screen {
            self.previous_screen = self.screen;
            self.screen = screen;
            self.overlay = Overlay::None;
            self.selected = 0;
            if screen == Screen::Settings {
                self.settings_selected = 0;
            }
        }
    }

    pub fn next_setting(&mut self) {
        const SETTINGS_COUNT: usize = 6;
        self.settings_selected = (self.settings_selected + 1) % SETTINGS_COUNT;
    }

    pub fn previous_setting(&mut self) {
        const SETTINGS_COUNT: usize = 6;
        self.settings_selected = (self.settings_selected + SETTINGS_COUNT - 1) % SETTINGS_COUNT;
    }

    pub fn first_setting(&mut self) {
        self.settings_selected = 0;
    }

    pub fn last_setting(&mut self) {
        self.settings_selected = 5;
    }

    pub fn open_details(&mut self) {
        if self.selected_device().is_some() {
            self.previous_screen = self.screen;
            self.screen = Screen::DeviceDetails;
            self.overlay = Overlay::None;
        }
    }

    pub fn close_details(&mut self) {
        self.screen = if self.previous_screen == Screen::DeviceDetails {
            Screen::Devices
        } else {
            self.previous_screen
        };
        self.overlay = Overlay::None;
    }

    pub fn begin_edit(&mut self, overlay: Overlay) {
        let Some(device) = self.selected_device() else {
            return;
        };
        self.editor = match overlay {
            Overlay::EditName => device.user.name.clone().unwrap_or_default(),
            Overlay::EditTags => device
                .user
                .tags
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(", "),
            Overlay::EditNotes => device.user.notes.clone(),
            _ => return,
        };
        self.overlay = overlay;
    }

    pub fn apply_editor(&mut self) {
        let overlay = self.overlay;
        let indices = self.filtered_indices();
        let Some(index) = indices.get(self.selected).copied() else {
            return;
        };
        let value = self.editor.trim().to_string();
        let Some(device) = self.devices.get_mut(index) else {
            return;
        };
        match overlay {
            Overlay::EditName => {
                device.user.name = (!value.is_empty()).then_some(value);
            }
            Overlay::EditTags => {
                device.user.tags = value
                    .split(',')
                    .map(str::trim)
                    .filter(|tag| !tag.is_empty())
                    .map(str::to_string)
                    .collect();
            }
            Overlay::EditNotes => device.user.notes = value,
            _ => return,
        }
        device.changed = true;
        device.last_changed = Utc::now();
        let event = TimelineEvent {
            id: Uuid::new_v4(),
            device_id: Some(device.id),
            occurred_at: Utc::now(),
            kind: EventKind::UserMetadataChanged,
            severity: Severity::Info,
            summary: format!("{} metadata updated", device.display_name()),
            detail: match overlay {
                Overlay::EditName => "User-provided display name changed",
                Overlay::EditTags => "User-provided tags changed",
                Overlay::EditNotes => "User-provided notes changed",
                _ => unreachable!(),
            }
            .into(),
        };
        self.events.push_front(event.clone());
        self.pending_persistence
            .push(PersistenceEvent::DeviceAndEvent {
                device: device.clone(),
                event,
            });
        self.toast("Device metadata saved", Severity::Info);
    }

    pub fn next_alert(&mut self) {
        if !self.alerts.is_empty() {
            self.selected = (self.selected + 1).min(self.alerts.len() - 1);
        }
    }

    pub fn previous_alert(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn first_alert(&mut self) {
        self.selected = 0;
    }

    pub fn last_alert(&mut self) {
        self.selected = self.alerts.len().saturating_sub(1);
    }

    pub fn acknowledge_selected_alert(&mut self) {
        let Some(alert) = self.alerts.get_mut(self.selected) else {
            return;
        };
        alert.acknowledged = true;
        self.pending_persistence
            .push(PersistenceEvent::Alert(alert.clone()));
        self.toast("Alert acknowledged", Severity::Info);
    }

    fn push_alert(
        &mut self,
        device_id: Option<Uuid>,
        severity: Severity,
        rule: &str,
        summary: impl Into<String>,
        detail: impl Into<String>,
    ) {
        let alert = Alert {
            id: Uuid::new_v4(),
            device_id,
            created_at: Utc::now(),
            resolved_at: None,
            severity,
            rule: rule.into(),
            summary: summary.into(),
            detail: detail.into(),
            acknowledged: false,
        };
        self.alerts.insert(0, alert.clone());
        self.pending_persistence
            .push(PersistenceEvent::Alert(alert));
    }

    pub fn cycle_filter(&mut self) {
        let current = DeviceFilter::ALL
            .iter()
            .position(|item| *item == self.filter)
            .unwrap_or(0);
        self.filter = DeviceFilter::ALL[(current + 1) % DeviceFilter::ALL.len()];
        self.selected = 0;
        self.toast(format!("Filter: {}", self.filter.label()), Severity::Info);
    }

    pub fn cycle_sort(&mut self) {
        let current = DeviceSort::ALL
            .iter()
            .position(|item| *item == self.sort)
            .unwrap_or(0);
        self.sort = DeviceSort::ALL[(current + 1) % DeviceSort::ALL.len()];
        self.sort_devices();
        self.toast(format!("Sorted by {}", self.sort.label()), Severity::Info);
    }

    pub fn cycle_theme(&mut self) {
        let current = ThemeName::ALL
            .iter()
            .position(|theme| *theme == self.theme)
            .unwrap_or(0);
        self.theme = ThemeName::ALL[(current + 1) % ThemeName::ALL.len()];
        self.toast(format!("Theme: {}", self.theme), Severity::Info);
    }

    pub fn cycle_icons(&mut self) {
        self.icons = match self.icons {
            IconMode::Nerd => IconMode::Unicode,
            IconMode::Unicode => IconMode::Ascii,
            IconMode::Ascii => IconMode::Nerd,
        };
        self.toast(format!("Icons: {:?}", self.icons), Severity::Info);
    }

    pub fn cycle_scan_mode(&mut self) {
        self.scan_mode = match self.scan_mode {
            ScanMode::Quick => ScanMode::Normal,
            ScanMode::Normal => ScanMode::Deep,
            ScanMode::Deep => ScanMode::Watch,
            ScanMode::Watch => ScanMode::Passive,
            ScanMode::Passive => ScanMode::Quick,
        };
        self.toast(
            format!("Scan mode: {}", self.scan_mode.label()),
            Severity::Info,
        );
    }

    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        if self
            .toast
            .as_ref()
            .is_some_and(|toast| toast.expires_at <= Utc::now())
        {
            self.toast = None;
        }
    }

    pub fn toast(&mut self, message: impl Into<String>, severity: Severity) {
        self.toast = Some(Toast {
            message: message.into(),
            severity,
            expires_at: Utc::now() + chrono::Duration::seconds(3),
        });
    }

    pub fn apply_discovery_event(&mut self, event: DiscoveryEvent) -> Option<PersistenceEvent> {
        match event {
            DiscoveryEvent::Started { spec, started_at } => {
                self.previous_snapshot = self.devices.iter().map(DeviceSnapshot::from).collect();
                self.scan = ScanState {
                    active: true,
                    id: Some(spec.id),
                    started_at: Some(started_at),
                    finished_at: None,
                    progress: Some(ScanProgress {
                        scan_id: spec.id,
                        phase: ScanPhase::Preparing,
                        completed: 0,
                        total: 0,
                        active: 0,
                        devices_found: 0,
                    }),
                    last_error: None,
                };
                self.log(
                    LogLevel::Info,
                    "scan",
                    "scan started",
                    format!("subnet={} mode={}", spec.subnet, spec.mode.label()),
                );
                None
            }
            DiscoveryEvent::Observation(observation) => self.merge_observation(observation),
            DiscoveryEvent::Progress(progress) => {
                self.scan.progress = Some(progress);
                None
            }
            DiscoveryEvent::ProviderHealth(health) => {
                if let Some(current) = self
                    .provider_health
                    .iter_mut()
                    .find(|item| item.provider == health.provider)
                {
                    *current = health;
                } else {
                    self.provider_health.push(health);
                }
                None
            }
            DiscoveryEvent::Finished {
                scan_id: _,
                cancelled,
            } => {
                self.scan.active = false;
                self.scan.finished_at = Some(Utc::now());
                let mut offline_events = Vec::new();
                if !cancelled {
                    self.scan_failures = 0;
                    if let Some(started_at) = self.scan.started_at {
                        offline_events = mark_unseen_offline(&mut self.devices, started_at);
                    }
                    let current = self
                        .devices
                        .iter()
                        .map(DeviceSnapshot::from)
                        .collect::<Vec<_>>();
                    self.scan_diff = diff_snapshots(&self.previous_snapshot, &current);
                    for event in &offline_events {
                        self.events.push_front(event.clone());
                    }
                    if self.baseline_active {
                        let missing = offline_events
                            .iter()
                            .filter_map(|event| event.device_id)
                            .filter_map(|id| self.devices.iter().find(|device| device.id == id))
                            .filter(|device| !device.baseline_unknown)
                            .map(|device| (device.id, device.display_name(), device.ipv4))
                            .collect::<Vec<_>>();
                        for (id, name, ip) in missing {
                            self.push_alert(
                                Some(id),
                                Severity::Warning,
                                "known_device_disappeared",
                                format!("Known device disappeared: {name}"),
                                format!("Expected baseline device at {ip} was not observed"),
                            );
                        }
                    }
                    if self.scan_diff.added.len() >= 10 {
                        self.push_alert(
                            None,
                            Severity::Warning,
                            "many_devices_appeared",
                            "Unusually many devices appeared",
                            format!(
                                "{} devices were added since the previous scan",
                                self.scan_diff.added.len()
                            ),
                        );
                    }
                }
                let found = self
                    .scan
                    .progress
                    .as_ref()
                    .map_or(0, |progress| progress.devices_found);
                self.activity
                    .push_back(u64::try_from(found).unwrap_or(u64::MAX));
                if self.activity.len() > 24 {
                    self.activity.pop_front();
                }
                self.toast(
                    if cancelled {
                        "Scan cancelled".into()
                    } else {
                        format!("Scan complete · {found} devices")
                    },
                    Severity::Info,
                );
                self.log(
                    LogLevel::Info,
                    "scan",
                    if cancelled {
                        "scan cancelled"
                    } else {
                        "scan complete"
                    },
                    format!("devices={found}"),
                );
                if offline_events.is_empty() {
                    Some(PersistenceEvent::ScanFinished)
                } else {
                    let values = offline_events
                        .into_iter()
                        .filter_map(|event| {
                            let device = event
                                .device_id
                                .and_then(|id| self.devices.iter().find(|device| device.id == id))?
                                .clone();
                            Some((device, event))
                        })
                        .collect();
                    Some(PersistenceEvent::DevicesAndEvents(values))
                }
            }
            DiscoveryEvent::Failed {
                scan_id: _,
                message,
            } => {
                self.scan.active = false;
                self.scan_failures = self.scan_failures.saturating_add(1);
                self.scan.last_error = Some(message.clone());
                self.error = Some(message.clone());
                self.overlay = Overlay::Error;
                self.log(LogLevel::Error, "scan", "scan failed", message);
                if self.scan_failures >= 3 {
                    self.push_alert(
                        None,
                        Severity::Warning,
                        "repeated_scan_failures",
                        "Repeated scan failures",
                        format!("{} consecutive scans have failed", self.scan_failures),
                    );
                }
                None
            }
        }
    }

    fn merge_observation(&mut self, mut observation: Observation) -> Option<PersistenceEvent> {
        if observation.vendor.is_none() {
            observation.vendor = observation
                .mac
                .as_deref()
                .and_then(|mac| self.vendor_database.as_ref()?.lookup(mac))
                .map(str::to_string);
        }
        let index = self.devices.iter().position(|device| {
            observation
                .mac
                .as_ref()
                .is_some_and(|mac| device.mac.as_ref() == Some(mac))
                || device.ipv4 == observation.ip
        });
        let (device_id, timeline, device) = if let Some(index) = index {
            let changes = self.devices[index].merge(&observation);
            let device = self.devices[index].clone();
            let timeline = changes
                .first()
                .map(|change| timeline_for_change(&device, change));
            (device.id, timeline, device)
        } else {
            let device = Device::from_observation(&observation);
            let timeline = TimelineEvent {
                id: Uuid::new_v4(),
                device_id: Some(device.id),
                occurred_at: observation.observed_at,
                kind: EventKind::DeviceNew,
                severity: if device.device_type == DeviceType::Unknown {
                    Severity::Warning
                } else {
                    Severity::Info
                },
                summary: format!("{} appeared", device.display_name()),
                detail: format!(
                    "First observed at {} by {}",
                    device.ipv4, observation.source
                ),
            };
            let id = device.id;
            self.devices.push(device.clone());
            (id, Some(timeline), device)
        };
        if let Some(timeline) = timeline {
            let mut created_alert = None;
            if timeline.kind == EventKind::DeviceNew && device.device_type == DeviceType::Unknown {
                let alert = Alert {
                    id: Uuid::new_v4(),
                    device_id: Some(device_id),
                    created_at: Utc::now(),
                    resolved_at: None,
                    severity: Severity::Warning,
                    rule: "unknown_device".into(),
                    summary: "Unknown device appeared".into(),
                    detail: format!("{} · first observed by {}", device.ipv4, observation.source),
                    acknowledged: false,
                };
                self.alerts.insert(0, alert.clone());
                created_alert = Some(alert);
            }
            self.events.push_front(timeline.clone());
            self.events.truncate(500);
            self.log(
                LogLevel::Info,
                "discovery",
                &timeline.summary,
                format!("ip={} source={}", observation.ip, observation.source),
            );
            if timeline.detail.contains("TCP service 23") {
                self.push_alert(
                    Some(device.id),
                    Severity::Warning,
                    "important_service_appeared",
                    format!("Telnet service appeared on {}", device.display_name()),
                    format!("{} exposes TCP 23 inside the local network", device.ipv4),
                );
            }
            if timeline.detail.contains("MAC address changed") {
                self.push_alert(
                    Some(device.id),
                    if matches!(device.device_type, DeviceType::Gateway | DeviceType::Router) {
                        Severity::Critical
                    } else {
                        Severity::Warning
                    },
                    "mac_changed",
                    format!("MAC address changed for {}", device.display_name()),
                    timeline.detail.clone(),
                );
            }
            self.sort_devices();
            Some(if let Some(alert) = created_alert {
                PersistenceEvent::DeviceEventAlert {
                    device,
                    event: timeline,
                    alert,
                }
            } else {
                PersistenceEvent::DeviceAndEvent {
                    device,
                    event: timeline,
                }
            })
        } else {
            Some(PersistenceEvent::Device(device))
        }
    }

    fn sort_devices(&mut self) {
        match self.sort {
            DeviceSort::Status => self.devices.sort_by_key(|device| {
                (
                    status_rank(device.status),
                    device.display_name().to_lowercase(),
                )
            }),
            DeviceSort::Name => self
                .devices
                .sort_by_key(|device| device.display_name().to_lowercase()),
            DeviceSort::Address => self.devices.sort_by_key(|device| u32::from(device.ipv4)),
            DeviceSort::Vendor => self
                .devices
                .sort_by_key(|device| device.vendor.clone().unwrap_or_default()),
            DeviceSort::Latency => self
                .devices
                .sort_by_key(|device| device.latency_ms.unwrap_or(u32::MAX)),
            DeviceSort::LastSeen => self
                .devices
                .sort_by_key(|device| std::cmp::Reverse(device.last_seen)),
            DeviceSort::Confidence => self
                .devices
                .sort_by_key(|device| std::cmp::Reverse(device.confidence)),
        }
        self.selected = self
            .selected
            .min(self.filtered_indices().len().saturating_sub(1));
    }

    fn log(&mut self, level: LogLevel, module: &str, message: &str, fields: String) {
        self.logs.push_back(LogEntry {
            at: Utc::now(),
            level,
            module: module.into(),
            message: message.into(),
            fields,
        });
        if self.logs.len() > 1_000 {
            self.logs.pop_front();
        }
    }

    #[must_use]
    pub fn counts(&self) -> DeviceCounts {
        let mut counts = DeviceCounts::default();
        for device in &self.devices {
            match device.status {
                DeviceStatus::Online => counts.online += 1,
                DeviceStatus::Offline => counts.offline += 1,
                DeviceStatus::Unknown | DeviceStatus::Scanning => counts.unknown_state += 1,
                DeviceStatus::Stale => counts.stale += 1,
            }
            if device.device_type == DeviceType::Unknown {
                counts.unknown += 1;
            }
            if device.changed {
                counts.changed += 1;
            }
        }
        counts
    }
}

#[derive(Debug, Clone)]
pub enum PersistenceEvent {
    Device(Device),
    DeviceAndEvent {
        device: Device,
        event: TimelineEvent,
    },
    DevicesAndEvents(Vec<(Device, TimelineEvent)>),
    DeviceEventAlert {
        device: Device,
        event: TimelineEvent,
        alert: Alert,
    },
    Alert(Alert),
    ScanFinished,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct DeviceCounts {
    pub online: usize,
    pub offline: usize,
    pub unknown: usize,
    pub unknown_state: usize,
    pub stale: usize,
    pub changed: usize,
}

pub struct AppRuntime {
    pub state: AppState,
    pub config: Config,
    discovery_tx: mpsc::Sender<DiscoveryEvent>,
    pub discovery_rx: mpsc::Receiver<DiscoveryEvent>,
    cancellation: Option<CancellationToken>,
    pub auto_scan: bool,
}

impl AppRuntime {
    #[must_use]
    pub fn new(state: AppState, config: Config) -> Self {
        let (discovery_tx, discovery_rx) = mpsc::channel(512);
        Self {
            state,
            config,
            discovery_tx,
            discovery_rx,
            cancellation: None,
            auto_scan: true,
        }
    }

    pub fn start_scan(&mut self) -> anyhow::Result<()> {
        if self.state.scan.active {
            self.state
                .toast("A scan is already running", Severity::Info);
            return Ok(());
        }
        let interface = self.state.active_interface.clone().ok_or_else(|| {
            anyhow::anyhow!("No IPv4 interface is available. Open `lantern doctor` for details.")
        })?;
        self.config.scan_mode = self.state.scan_mode;
        let spec = ScanSpec::from_config(&self.config, interface)?;
        let token = CancellationToken::new();
        self.cancellation = Some(token.clone());
        self.state.scan.active = true;
        self.state.scan.started_at = Some(Utc::now());
        self.state.scan.last_error = None;
        spawn_scan(spec, self.discovery_tx.clone(), token);
        Ok(())
    }

    pub fn cancel_scan(&mut self) {
        if let Some(token) = self.cancellation.take() {
            token.cancel();
        }
    }

    pub async fn shutdown(&mut self) {
        self.cancel_scan();
        tokio::task::yield_now().await;
    }
}

fn timeline_for_change(device: &Device, change: &DeviceChange) -> TimelineEvent {
    let (kind, summary, detail) = match change {
        DeviceChange::Identity(detail) => (
            EventKind::IdentityChanged,
            format!("{} identity changed", device.display_name()),
            detail.clone(),
        ),
        DeviceChange::ServiceAdded(port) => (
            EventKind::ServicesChanged,
            format!("{} services changed", device.display_name()),
            format!("TCP service {port} was added"),
        ),
        DeviceChange::ServiceRemoved(port) => (
            EventKind::ServicesChanged,
            format!("{} services changed", device.display_name()),
            format!("TCP service {port} was removed"),
        ),
        DeviceChange::AddressChanged { from, to } => (
            EventKind::AddressChanged,
            format!("{} address changed", device.display_name()),
            format!("{from} -> {to}"),
        ),
        DeviceChange::Fingerprint { from, to } => (
            EventKind::IdentityChanged,
            format!("{} inference changed", device.display_name()),
            format!("{from} -> {to}"),
        ),
        DeviceChange::Status { from, to } => (
            if *to == DeviceStatus::Offline {
                EventKind::DeviceOffline
            } else {
                EventKind::DeviceOnline
            },
            format!("{} is {to}", device.display_name()),
            format!("{from} -> {to}"),
        ),
    };
    TimelineEvent {
        id: Uuid::new_v4(),
        device_id: Some(device.id),
        occurred_at: Utc::now(),
        kind,
        severity: Severity::Info,
        summary,
        detail,
    }
}

const fn status_rank(status: DeviceStatus) -> u8 {
    match status {
        DeviceStatus::Online => 0,
        DeviceStatus::Scanning => 1,
        DeviceStatus::Unknown => 2,
        DeviceStatus::Offline => 3,
        DeviceStatus::Stale => 4,
    }
}

#[must_use]
pub fn demo_state(config: &Config) -> AppState {
    let now = Utc::now();
    let descriptions = [
        (
            [192, 168, 1, 1],
            "Home gateway",
            Some("Ubiquiti"),
            &[80, 443][..],
            DeviceType::Gateway,
        ),
        (
            [192, 168, 1, 20],
            "Home NAS",
            Some("Synology"),
            &[22, 443, 445, 5000, 5001][..],
            DeviceType::Nas,
        ),
        (
            [192, 168, 1, 25],
            "Studio desktop",
            Some("Dell"),
            &[22, 445][..],
            DeviceType::Computer,
        ),
        (
            [192, 168, 1, 48],
            "Living room TV",
            Some("Samsung"),
            &[8008, 8009][..],
            DeviceType::SmartTv,
        ),
        (
            [192, 168, 1, 70],
            "Office printer",
            Some("Brother"),
            &[631, 9100][..],
            DeviceType::Printer,
        ),
        (
            [192, 168, 1, 91],
            "Unknown device",
            None,
            &[443][..],
            DeviceType::Unknown,
        ),
    ];
    let mut devices = Vec::new();
    for (number, (ip, name, vendor, ports, kind)) in descriptions.iter().enumerate() {
        let mut first = Observation::tcp(
            Ipv4Addr::from(*ip),
            ports[0],
            std::time::Duration::from_millis((number as u64 + 1) * 2),
            "en0",
            "192.168.1.0/24",
        );
        first.hostname = Some((*name).into());
        first.vendor = vendor.map(str::to_string);
        first.mac = Some(format!("02:00:00:00:00:{:02x}", ip[3]));
        first.observed_at = now - chrono::Duration::minutes(number as i64 * 3);
        let mut device = Device::from_observation(&first);
        for port in ports.iter().skip(1) {
            device.merge(&Observation::tcp(
                device.ipv4,
                *port,
                std::time::Duration::from_millis(number as u64 + 1),
                "en0",
                "192.168.1.0/24",
            ));
        }
        device.device_type = *kind;
        device.confidence = if *kind == DeviceType::Unknown {
            18
        } else {
            76 + number as u8 * 3
        };
        if number == 4 {
            device.status = DeviceStatus::Offline;
        }
        devices.push(device);
    }
    let interface = NetworkInterface {
        name: "en0".into(),
        address: Ipv4Addr::new(192, 168, 1, 15),
        netmask: Ipv4Addr::new(255, 255, 255, 0),
        subnet: "192.168.1.0/24".parse().expect("valid demo subnet"),
        is_loopback: false,
        likely_active: true,
    };
    let mut state = AppState::new(config, vec![interface], devices);
    state.events = VecDeque::from([
        TimelineEvent {
            id: Uuid::new_v4(),
            device_id: None,
            occurred_at: now - chrono::Duration::minutes(2),
            kind: EventKind::DeviceNew,
            severity: Severity::Warning,
            summary: "Living room TV appeared".into(),
            detail: "First observed by TCP and mDNS · 192.168.1.48".into(),
        },
        TimelineEvent {
            id: Uuid::new_v4(),
            device_id: None,
            occurred_at: now - chrono::Duration::minutes(5),
            kind: EventKind::ServicesChanged,
            severity: Severity::Info,
            summary: "Home NAS services changed".into(),
            detail: "HTTPS 443 added".into(),
        },
        TimelineEvent {
            id: Uuid::new_v4(),
            device_id: None,
            occurred_at: now - chrono::Duration::minutes(9),
            kind: EventKind::DeviceOffline,
            severity: Severity::Info,
            summary: "Office printer went offline".into(),
            detail: "Last seen 8 minutes ago".into(),
        },
    ]);
    state.alerts = vec![
        Alert {
            id: Uuid::new_v4(),
            device_id: None,
            created_at: now - chrono::Duration::minutes(2),
            resolved_at: None,
            severity: Severity::Warning,
            rule: "unknown_device".into(),
            summary: "Unknown device appeared".into(),
            detail: "192.168.1.91 · first seen today".into(),
            acknowledged: false,
        },
        Alert {
            id: Uuid::new_v4(),
            device_id: None,
            created_at: now - chrono::Duration::minutes(12),
            resolved_at: None,
            severity: Severity::Critical,
            rule: "gateway_mac".into(),
            summary: "Gateway MAC address changed".into(),
            detail: "Review the old and new observed addresses".into(),
            acknowledged: false,
        },
    ];
    state.activity = VecDeque::from([
        12, 12, 13, 14, 16, 18, 17, 20, 19, 21, 23, 22, 24, 24, 25, 24,
    ]);
    state
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_prefers_lan_interface_over_container_bridge() {
        let bridge = NetworkInterface {
            name: "br-test".into(),
            address: Ipv4Addr::new(172, 18, 0, 1),
            netmask: Ipv4Addr::new(255, 255, 0, 0),
            subnet: "172.18.0.0/16".parse().expect("bridge subnet"),
            is_loopback: false,
            likely_active: true,
        };
        let lan = NetworkInterface {
            name: "eth-test".into(),
            address: Ipv4Addr::new(192, 168, 40, 80),
            netmask: Ipv4Addr::new(255, 255, 255, 0),
            subnet: "192.168.40.0/24".parse().expect("LAN subnet"),
            is_loopback: false,
            likely_active: true,
        };
        let state = AppState::new(&Config::default(), vec![bridge, lan], Vec::new());
        assert_eq!(
            state
                .active_interface
                .as_ref()
                .map(|interface| interface.name.as_str()),
            Some("eth-test")
        );
    }

    #[test]
    fn vendor_database_enriches_new_mac_observations() {
        let database = VendorDatabase::parse("00:11:22 Synology Inc.").expect("vendor database");
        let mut state = AppState::new(&Config::default(), Vec::new(), Vec::new());
        state.set_vendor_database(database);
        let mut observation = Observation::tcp(
            Ipv4Addr::new(192, 168, 40, 20),
            443,
            std::time::Duration::from_millis(2),
            "eth-test",
            "192.168.40.0/24",
        );
        observation.mac = Some("00:11:22:33:44:55".into());
        state.merge_observation(observation);
        let device = state.devices.first().expect("discovered device");
        assert_eq!(device.vendor.as_deref(), Some("Synology Inc."));
        assert_eq!(device.device_type, DeviceType::Nas);
        assert!(device.display_name().contains("Synology"));
    }

    #[test]
    fn loading_vendor_database_refreshes_stored_fingerprints() {
        let mut device = Device::from_observation(&Observation {
            ip: Ipv4Addr::new(192, 168, 40, 23),
            mac: None,
            hostname: Some("iPhone.local".into()),
            vendor: None,
            open_port: None,
            latency: None,
            source: crate::devices::DiscoverySource::History,
            interface: "eth-test".into(),
            subnet: "192.168.40.0/24".into(),
            observed_at: Utc::now(),
            http: None,
        });
        device.mac = Some("00:11:22:33:44:55".into());
        device.vendor = Some("Synology Inc., stale address".into());
        device.device_type = DeviceType::Unknown;
        device.confidence = 0;
        let database = VendorDatabase::parse("00:11:22 Synology Inc.").expect("vendor database");
        let mut state = AppState::new(&Config::default(), Vec::new(), vec![device]);

        state.set_vendor_database(database);

        assert_eq!(state.devices[0].vendor.as_deref(), Some("Synology Inc."));
        assert_eq!(state.devices[0].device_type, DeviceType::Phone);
        assert_eq!(state.devices[0].confidence, 65);
        assert_eq!(state.pending_persistence.len(), 1);
    }

    #[test]
    fn navigation_never_exceeds_filtered_results() {
        let config = Config::default();
        let mut state = demo_state(&config);
        state.filter = DeviceFilter::Offline;
        for _ in 0..10 {
            state.next();
        }
        assert_eq!(state.selected, 0);
        assert!(state.selected_device().is_some());
    }

    #[test]
    fn settings_navigation_wraps_independently_from_device_selection() {
        let mut state = demo_state(&Config::default());
        state.change_screen(Screen::Settings);
        assert_eq!(state.settings_selected, 0);

        state.previous_setting();
        assert_eq!(state.settings_selected, 5);
        state.next_setting();
        assert_eq!(state.settings_selected, 0);
        state.last_setting();
        assert_eq!(state.settings_selected, 5);
        state.first_setting();
        assert_eq!(state.settings_selected, 0);
    }

    #[test]
    fn theme_cycle_visits_every_palette_and_wraps() {
        let mut state = demo_state(&Config::default());
        let initial = state.theme;
        let mut visited = Vec::new();
        for _ in 0..ThemeName::ALL.len() {
            state.cycle_theme();
            visited.push(state.theme);
        }
        assert_eq!(state.theme, initial);
        for theme in ThemeName::ALL {
            assert!(visited.contains(&theme), "cycle skipped {theme}");
        }
    }

    #[test]
    fn fuzzy_search_matches_service_and_vendor() {
        let config = Config::default();
        let mut state = demo_state(&config);
        state.search = "synol nas".into();
        assert_eq!(state.filtered_indices().len(), 1);
        state.search = "definitely-not-present".into();
        assert!(state.filtered_indices().is_empty());
        state.search = "printer".into();
        assert_eq!(state.filtered_indices().len(), 1);
    }

    #[test]
    fn overlay_does_not_change_screen() {
        let config = Config::default();
        let mut state = demo_state(&config);
        state.screen = Screen::Devices;
        state.overlay = Overlay::Help;
        assert_eq!(state.screen, Screen::Devices);
    }

    #[test]
    fn user_metadata_edit_is_separate_and_persisted() {
        let config = Config::default();
        let mut state = demo_state(&config);
        let observed_hostname = state
            .selected_device()
            .and_then(|device| device.hostname.clone());
        state.begin_edit(Overlay::EditName);
        state.editor = "Trusted gateway".into();
        state.apply_editor();
        let device = state.selected_device().expect("selected device");
        assert_eq!(device.user.name.as_deref(), Some("Trusted gateway"));
        assert_eq!(device.hostname, observed_hostname);
        assert!(matches!(
            state.pending_persistence.first(),
            Some(PersistenceEvent::DeviceAndEvent { .. })
        ));
    }
}
