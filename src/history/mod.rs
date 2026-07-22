use std::{collections::BTreeMap, path::Path};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    alerts::{Alert, Severity},
    devices::{Device, DeviceStatus},
};

const MIGRATION_1: &str = r#"
PRAGMA foreign_keys = ON;
CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER PRIMARY KEY,
    applied_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS scans (
    id TEXT PRIMARY KEY,
    started_at TEXT NOT NULL,
    finished_at TEXT,
    interface TEXT NOT NULL,
    subnet TEXT NOT NULL,
    mode TEXT NOT NULL,
    status TEXT NOT NULL,
    summary_json TEXT NOT NULL DEFAULT '{}'
);
CREATE TABLE IF NOT EXISTS devices (
    id TEXT PRIMARY KEY,
    stable_key TEXT NOT NULL UNIQUE,
    first_seen TEXT NOT NULL,
    last_seen TEXT NOT NULL,
    last_changed TEXT NOT NULL,
    user_name TEXT,
    inferred_type TEXT NOT NULL,
    confidence INTEGER NOT NULL,
    tags_json TEXT NOT NULL DEFAULT '[]',
    notes TEXT NOT NULL DEFAULT '',
    snapshot_json TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS addresses (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    device_id TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    address TEXT NOT NULL,
    mac TEXT,
    first_seen TEXT NOT NULL,
    last_seen TEXT NOT NULL,
    is_current INTEGER NOT NULL,
    UNIQUE(device_id, kind, address)
);
CREATE TABLE IF NOT EXISTS observations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    scan_id TEXT REFERENCES scans(id) ON DELETE SET NULL,
    device_id TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    observed_at TEXT NOT NULL,
    source TEXT NOT NULL,
    payload_json TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS services (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    device_id TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    transport TEXT NOT NULL,
    port INTEGER NOT NULL,
    name TEXT NOT NULL,
    metadata_json TEXT NOT NULL DEFAULT '{}',
    first_seen TEXT NOT NULL,
    last_seen TEXT NOT NULL,
    is_current INTEGER NOT NULL,
    UNIQUE(device_id, transport, port)
);
CREATE TABLE IF NOT EXISTS events (
    id TEXT PRIMARY KEY,
    device_id TEXT REFERENCES devices(id) ON DELETE SET NULL,
    scan_id TEXT REFERENCES scans(id) ON DELETE SET NULL,
    occurred_at TEXT NOT NULL,
    kind TEXT NOT NULL,
    severity TEXT NOT NULL,
    summary TEXT NOT NULL,
    details_json TEXT NOT NULL DEFAULT '{}'
);
CREATE TABLE IF NOT EXISTS baselines (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    created_at TEXT NOT NULL,
    subnet TEXT NOT NULL,
    snapshot_json TEXT NOT NULL,
    is_active INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS alerts (
    id TEXT PRIMARY KEY,
    device_id TEXT REFERENCES devices(id) ON DELETE SET NULL,
    event_id TEXT REFERENCES events(id) ON DELETE SET NULL,
    created_at TEXT NOT NULL,
    resolved_at TEXT,
    severity TEXT NOT NULL,
    rule TEXT NOT NULL,
    summary TEXT NOT NULL,
    acknowledged INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_devices_last_seen ON devices(last_seen DESC);
CREATE INDEX IF NOT EXISTS idx_events_time ON events(occurred_at DESC);
CREATE INDEX IF NOT EXISTS idx_events_device ON events(device_id, occurred_at DESC);
CREATE INDEX IF NOT EXISTS idx_addresses_current ON addresses(device_id, is_current);
CREATE INDEX IF NOT EXISTS idx_alerts_open ON alerts(resolved_at, severity);
INSERT OR IGNORE INTO schema_migrations(version, applied_at) VALUES (1, datetime('now'));
"#;

const MIGRATION_2: &str = r#"
ALTER TABLE alerts ADD COLUMN detail TEXT NOT NULL DEFAULT '';
INSERT INTO schema_migrations(version, applied_at) VALUES (2, datetime('now'));
"#;

#[derive(Debug)]
pub struct HistoryStore {
    connection: Connection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimelineEvent {
    pub id: Uuid,
    pub device_id: Option<Uuid>,
    pub occurred_at: DateTime<Utc>,
    pub kind: EventKind,
    pub severity: Severity,
    pub summary: String,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    DeviceNew,
    DeviceOnline,
    DeviceOffline,
    IdentityChanged,
    AddressChanged,
    ServicesChanged,
    UserMetadataChanged,
    ScanFailed,
    BaselineDifference,
}

impl EventKind {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::DeviceNew => "NEW",
            Self::DeviceOnline => "ONLINE",
            Self::DeviceOffline => "OFFLINE",
            Self::IdentityChanged => "IDENTITY",
            Self::AddressChanged => "ADDRESS",
            Self::ServicesChanged => "CHANGED",
            Self::UserMetadataChanged => "EDITED",
            Self::ScanFailed => "FAILED",
            Self::BaselineDifference => "BASELINE",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanDiff {
    pub added: Vec<DeviceSnapshot>,
    pub removed: Vec<DeviceSnapshot>,
    pub changed: Vec<DeviceDifference>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceSnapshot {
    pub stable_key: String,
    pub name: String,
    pub ip: String,
    pub mac: Option<String>,
    pub services: Vec<u16>,
}

impl From<&Device> for DeviceSnapshot {
    fn from(device: &Device) -> Self {
        Self {
            stable_key: device.stable_key.clone(),
            name: device.display_name(),
            ip: device.ipv4.to_string(),
            mac: device.mac.clone(),
            services: device.services.keys().copied().collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceDifference {
    pub before: DeviceSnapshot,
    pub after: DeviceSnapshot,
    pub changes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Baseline {
    pub id: Uuid,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub subnet: String,
    pub devices: Vec<DeviceSnapshot>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaselineDiff {
    pub unknown: Vec<DeviceSnapshot>,
    pub missing: Vec<DeviceSnapshot>,
    pub changed: Vec<DeviceDifference>,
}

impl HistoryStore {
    pub fn open(path: &Path) -> Result<Self> {
        let connection = Connection::open(path)
            .with_context(|| format!("open history database at {}", path.display()))?;
        let mut store = Self { connection };
        store.migrate()?;
        Ok(store)
    }

    #[cfg(test)]
    fn memory() -> Result<Self> {
        let connection = Connection::open_in_memory().context("open in-memory database")?;
        let mut store = Self { connection };
        store.migrate()?;
        Ok(store)
    }

    pub fn migrate(&mut self) -> Result<()> {
        self.connection
            .execute_batch(MIGRATION_1)
            .context("apply database migration 1")?;
        let version: i64 = self.connection.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |row| row.get(0),
        )?;
        if version < 2 {
            self.connection
                .execute_batch(MIGRATION_2)
                .context("apply database migration 2")?;
        }
        Ok(())
    }

    pub fn upsert_device(&mut self, device: &Device) -> Result<()> {
        let transaction = self
            .connection
            .transaction()
            .context("start device transaction")?;
        let snapshot = serde_json::to_string(device).context("serialize device")?;
        let tags = serde_json::to_string(&device.user.tags).context("serialize tags")?;
        transaction.execute(
            r#"INSERT INTO devices
               (id, stable_key, first_seen, last_seen, last_changed, user_name, inferred_type,
                confidence, tags_json, notes, snapshot_json)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
               ON CONFLICT(id) DO UPDATE SET
                 stable_key=excluded.stable_key,
                 last_seen=excluded.last_seen,
                 last_changed=excluded.last_changed,
                 user_name=excluded.user_name,
                 inferred_type=excluded.inferred_type,
                 confidence=excluded.confidence,
                 tags_json=excluded.tags_json,
                 notes=excluded.notes,
                 snapshot_json=excluded.snapshot_json"#,
            params![
                device.id.to_string(),
                device.stable_key,
                device.first_seen.to_rfc3339(),
                device.last_seen.to_rfc3339(),
                device.last_changed.to_rfc3339(),
                device.user.name,
                device.device_type.label(),
                device.confidence,
                tags,
                device.user.notes,
                snapshot,
            ],
        )?;
        transaction.execute(
            r#"INSERT INTO addresses
               (device_id, kind, address, mac, first_seen, last_seen, is_current)
               VALUES (?1, 'ipv4', ?2, ?3, ?4, ?5, 1)
               ON CONFLICT(device_id, kind, address) DO UPDATE SET
                 mac=COALESCE(excluded.mac, mac), last_seen=excluded.last_seen, is_current=1"#,
            params![
                device.id.to_string(),
                device.ipv4.to_string(),
                device.mac,
                device.first_seen.to_rfc3339(),
                device.last_seen.to_rfc3339(),
            ],
        )?;
        for service in device.services.values() {
            transaction.execute(
                r#"INSERT INTO services
                   (device_id, transport, port, name, first_seen, last_seen, is_current)
                   VALUES (?1, 'tcp', ?2, ?3, ?4, ?5, 1)
                   ON CONFLICT(device_id, transport, port) DO UPDATE SET
                     name=excluded.name, last_seen=excluded.last_seen, is_current=1"#,
                params![
                    device.id.to_string(),
                    service.port,
                    service.name,
                    service.first_seen.to_rfc3339(),
                    service.last_seen.to_rfc3339(),
                ],
            )?;
        }
        transaction.commit().context("commit device transaction")
    }

    pub fn load_devices(&self) -> Result<Vec<Device>> {
        let mut statement = self
            .connection
            .prepare("SELECT snapshot_json FROM devices ORDER BY last_seen DESC")?;
        let values = statement.query_map([], |row| row.get::<_, String>(0))?;
        let mut devices = Vec::new();
        for value in values {
            let json = value?;
            match serde_json::from_str::<Device>(&json) {
                Ok(device) => devices.push(device),
                Err(error) => tracing::warn!(%error, "ignored unreadable device snapshot"),
            }
        }
        Ok(devices)
    }

    pub fn record_event(&self, event: &TimelineEvent) -> Result<()> {
        self.connection.execute(
            r#"INSERT OR REPLACE INTO events
               (id, device_id, occurred_at, kind, severity, summary, details_json)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"#,
            params![
                event.id.to_string(),
                event.device_id.map(|id| id.to_string()),
                event.occurred_at.to_rfc3339(),
                format!("{:?}", event.kind),
                format!("{:?}", event.severity),
                event.summary,
                serde_json::json!({ "detail": event.detail }).to_string(),
            ],
        )?;
        Ok(())
    }

    pub fn load_events(&self, limit: usize) -> Result<Vec<TimelineEvent>> {
        let mut statement = self.connection.prepare(
            "SELECT id, device_id, occurred_at, kind, severity, summary, details_json FROM events ORDER BY occurred_at DESC LIMIT ?1",
        )?;
        let rows = statement.query_map([i64::try_from(limit).unwrap_or(i64::MAX)], |row| {
            let id: String = row.get(0)?;
            let device_id: Option<String> = row.get(1)?;
            let occurred_at: String = row.get(2)?;
            let kind: String = row.get(3)?;
            let severity: String = row.get(4)?;
            let details: String = row.get(6)?;
            let detail = serde_json::from_str::<serde_json::Value>(&details)
                .ok()
                .and_then(|value| {
                    value
                        .get("detail")
                        .and_then(|item| item.as_str())
                        .map(str::to_string)
                })
                .unwrap_or_default();
            Ok((
                id,
                device_id,
                occurred_at,
                kind,
                severity,
                row.get::<_, String>(5)?,
                detail,
            ))
        })?;
        let mut events = Vec::new();
        for row in rows {
            let (id, device_id, occurred_at, kind, severity, summary, detail) = row?;
            let Ok(id) = Uuid::parse_str(&id) else {
                continue;
            };
            let Ok(occurred_at) = DateTime::parse_from_rfc3339(&occurred_at) else {
                continue;
            };
            events.push(TimelineEvent {
                id,
                device_id: device_id.and_then(|value| Uuid::parse_str(&value).ok()),
                occurred_at: occurred_at.with_timezone(&Utc),
                kind: parse_event_kind(&kind),
                severity: parse_severity(&severity),
                summary,
                detail,
            });
        }
        Ok(events)
    }

    pub fn record_alert(&self, alert: &Alert) -> Result<()> {
        self.connection.execute(
            r#"INSERT OR REPLACE INTO alerts
               (id, device_id, created_at, resolved_at, severity, rule, summary, acknowledged, detail)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"#,
            params![
                alert.id.to_string(),
                alert.device_id.map(|id| id.to_string()),
                alert.created_at.to_rfc3339(),
                alert.resolved_at.map(|at| at.to_rfc3339()),
                format!("{:?}", alert.severity),
                alert.rule,
                alert.summary,
                alert.acknowledged,
                alert.detail,
            ],
        )?;
        Ok(())
    }

    pub fn load_alerts(&self) -> Result<Vec<Alert>> {
        let mut statement = self.connection.prepare(
            "SELECT id, device_id, created_at, resolved_at, severity, rule, summary, acknowledged, detail FROM alerts ORDER BY resolved_at IS NULL DESC, created_at DESC",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, bool>(7)?,
                row.get::<_, String>(8)?,
            ))
        })?;
        let mut alerts = Vec::new();
        for row in rows {
            let (
                id,
                device_id,
                created_at,
                resolved_at,
                severity,
                rule,
                summary,
                acknowledged,
                detail,
            ) = row?;
            let (Ok(id), Ok(created_at)) = (
                Uuid::parse_str(&id),
                DateTime::parse_from_rfc3339(&created_at),
            ) else {
                continue;
            };
            alerts.push(Alert {
                id,
                device_id: device_id.and_then(|value| Uuid::parse_str(&value).ok()),
                created_at: created_at.with_timezone(&Utc),
                resolved_at: resolved_at
                    .and_then(|value| DateTime::parse_from_rfc3339(&value).ok())
                    .map(|value| value.with_timezone(&Utc)),
                severity: parse_severity(&severity),
                rule,
                summary,
                detail,
                acknowledged,
            });
        }
        Ok(alerts)
    }

    pub fn prune(&self, retention_days: u32) -> Result<usize> {
        let cutoff = (Utc::now() - chrono::Duration::days(i64::from(retention_days))).to_rfc3339();
        let mut removed = 0;
        removed += self
            .connection
            .execute("DELETE FROM observations WHERE observed_at < ?1", [&cutoff])?;
        removed += self
            .connection
            .execute("DELETE FROM events WHERE occurred_at < ?1", [&cutoff])?;
        removed += self.connection.execute(
            "DELETE FROM scans WHERE finished_at IS NOT NULL AND finished_at < ?1",
            [&cutoff],
        )?;
        removed += self.connection.execute(
            "DELETE FROM addresses WHERE is_current=0 AND last_seen < ?1",
            [&cutoff],
        )?;
        removed += self.connection.execute(
            "DELETE FROM services WHERE is_current=0 AND last_seen < ?1",
            [&cutoff],
        )?;
        Ok(removed)
    }

    pub fn save_baseline(&self, name: &str, subnet: &str, devices: &[Device]) -> Result<Baseline> {
        let baseline = Baseline {
            id: Uuid::new_v4(),
            name: name.into(),
            created_at: Utc::now(),
            subnet: subnet.into(),
            devices: devices.iter().map(DeviceSnapshot::from).collect(),
        };
        self.connection
            .execute("UPDATE baselines SET is_active=0", [])?;
        self.connection.execute(
            "INSERT INTO baselines(id, name, created_at, subnet, snapshot_json, is_active) VALUES (?1, ?2, ?3, ?4, ?5, 1)",
            params![
                baseline.id.to_string(), baseline.name, baseline.created_at.to_rfc3339(),
                baseline.subnet, serde_json::to_string(&baseline)?
            ],
        )?;
        Ok(baseline)
    }

    pub fn active_baseline(&self) -> Result<Option<Baseline>> {
        let json = self
            .connection
            .query_row(
                "SELECT snapshot_json FROM baselines WHERE is_active=1 ORDER BY created_at DESC LIMIT 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        json.map(|value| serde_json::from_str(&value).context("parse baseline"))
            .transpose()
    }
}

fn parse_event_kind(value: &str) -> EventKind {
    match value {
        "DeviceNew" => EventKind::DeviceNew,
        "DeviceOnline" => EventKind::DeviceOnline,
        "DeviceOffline" => EventKind::DeviceOffline,
        "IdentityChanged" => EventKind::IdentityChanged,
        "AddressChanged" => EventKind::AddressChanged,
        "ServicesChanged" => EventKind::ServicesChanged,
        "UserMetadataChanged" => EventKind::UserMetadataChanged,
        "ScanFailed" => EventKind::ScanFailed,
        "BaselineDifference" => EventKind::BaselineDifference,
        _ => EventKind::IdentityChanged,
    }
}

fn parse_severity(value: &str) -> Severity {
    match value {
        "Warning" => Severity::Warning,
        "Critical" => Severity::Critical,
        _ => Severity::Info,
    }
}

#[must_use]
pub fn diff_snapshots(before: &[DeviceSnapshot], after: &[DeviceSnapshot]) -> ScanDiff {
    let before_map: BTreeMap<&str, &DeviceSnapshot> = before
        .iter()
        .map(|item| (item.stable_key.as_str(), item))
        .collect();
    let after_map: BTreeMap<&str, &DeviceSnapshot> = after
        .iter()
        .map(|item| (item.stable_key.as_str(), item))
        .collect();
    let added = after_map
        .iter()
        .filter(|(key, _)| !before_map.contains_key(*key))
        .map(|(_, value)| (*value).clone())
        .collect();
    let removed = before_map
        .iter()
        .filter(|(key, _)| !after_map.contains_key(*key))
        .map(|(_, value)| (*value).clone())
        .collect();
    let mut changed = Vec::new();
    for (key, old) in &before_map {
        let Some(new) = after_map.get(key) else {
            continue;
        };
        let mut changes = Vec::new();
        if old.ip != new.ip {
            changes.push(format!("IP {} -> {}", old.ip, new.ip));
        }
        if old.mac != new.mac {
            changes.push("MAC address changed".into());
        }
        for port in new
            .services
            .iter()
            .filter(|port| !old.services.contains(port))
        {
            changes.push(format!("service {port} added"));
        }
        for port in old
            .services
            .iter()
            .filter(|port| !new.services.contains(port))
        {
            changes.push(format!("service {port} removed"));
        }
        if !changes.is_empty() {
            changed.push(DeviceDifference {
                before: (*old).clone(),
                after: (*new).clone(),
                changes,
            });
        }
    }
    ScanDiff {
        added,
        removed,
        changed,
    }
}

#[must_use]
pub fn compare_baseline(baseline: &Baseline, current: &[Device]) -> BaselineDiff {
    let current: Vec<DeviceSnapshot> = current.iter().map(DeviceSnapshot::from).collect();
    let diff = diff_snapshots(&baseline.devices, &current);
    BaselineDiff {
        unknown: diff.added,
        missing: diff.removed,
        changed: diff.changed,
    }
}

#[must_use]
pub fn mark_unseen_offline(
    devices: &mut [Device],
    scan_started: DateTime<Utc>,
) -> Vec<TimelineEvent> {
    let mut events = Vec::new();
    for device in devices {
        if device.last_seen < scan_started && device.status == DeviceStatus::Online {
            device.status = DeviceStatus::Offline;
            device.changed = true;
            device.last_changed = Utc::now();
            events.push(TimelineEvent {
                id: Uuid::new_v4(),
                device_id: Some(device.id),
                occurred_at: Utc::now(),
                kind: EventKind::DeviceOffline,
                severity: Severity::Info,
                summary: format!("{} went offline", device.display_name()),
                detail: format!("Last positive observation was at {}", device.last_seen),
            });
        }
    }
    events
}

#[cfg(test)]
mod tests {
    use std::{net::Ipv4Addr, time::Duration};

    use crate::devices::Observation;

    use super::*;

    fn device(ip: [u8; 4], port: u16) -> Device {
        Device::from_observation(&Observation::tcp(
            Ipv4Addr::from(ip),
            port,
            Duration::from_millis(1),
            "test0",
            "192.0.2.0/24",
        ))
    }

    #[test]
    fn migration_and_device_round_trip() {
        let mut store = HistoryStore::memory().expect("database");
        store.migrate().expect("repeat migration");
        let value = device([192, 0, 2, 8], 443);
        store.upsert_device(&value).expect("store");
        let loaded = store.load_devices().expect("load");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].ipv4, value.ipv4);
    }

    #[test]
    fn alerts_round_trip_and_keep_acknowledgement() {
        let store = HistoryStore::memory().expect("database");
        let alert = Alert {
            id: Uuid::new_v4(),
            device_id: None,
            created_at: Utc::now(),
            resolved_at: None,
            severity: Severity::Warning,
            rule: "test".into(),
            summary: "Unknown device".into(),
            detail: "192.0.2.3".into(),
            acknowledged: true,
        };
        store.record_alert(&alert).expect("record alert");
        let loaded = store.load_alerts().expect("load alerts");
        assert_eq!(loaded.len(), 1);
        assert!(loaded[0].acknowledged);
        assert_eq!(loaded[0].detail, "192.0.2.3");
    }

    #[test]
    fn diff_reports_added_removed_and_service_changes() {
        let a = device([192, 0, 2, 8], 80);
        let b = device([192, 0, 2, 9], 22);
        let before = vec![DeviceSnapshot::from(&a), DeviceSnapshot::from(&b)];
        let mut a_after = a.clone();
        a_after.merge(&Observation::tcp(
            a.ipv4,
            443,
            Duration::from_millis(1),
            "test0",
            "192.0.2.0/24",
        ));
        let c = device([192, 0, 2, 10], 9100);
        let after = vec![DeviceSnapshot::from(&a_after), DeviceSnapshot::from(&c)];
        let diff = diff_snapshots(&before, &after);
        assert_eq!(diff.added.len(), 1);
        assert_eq!(diff.removed.len(), 1);
        assert_eq!(diff.changed.len(), 1);
    }

    #[test]
    fn baseline_flags_unknown_and_missing_devices() {
        let expected = device([192, 0, 2, 8], 443);
        let unexpected = device([192, 0, 2, 9], 22);
        let baseline = Baseline {
            id: Uuid::new_v4(),
            name: "test".into(),
            created_at: Utc::now(),
            subnet: "192.0.2.0/24".into(),
            devices: vec![DeviceSnapshot::from(&expected)],
        };
        let diff = compare_baseline(&baseline, &[unexpected]);
        assert_eq!(diff.unknown.len(), 1);
        assert_eq!(diff.missing.len(), 1);
    }
}
