use std::io::Write;

use anyhow::{Context, Result};
use quick_xml::se::Serializer;
use serde::Serialize;

use crate::{
    alerts::{Alert, Severity},
    devices::Device,
    history::{Baseline, BaselineDiff, DeviceSnapshot, EventKind, TimelineEvent},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Json,
    Xml,
    Csv,
}

pub fn devices<W: Write>(values: &[Device], format: Format, mut writer: W) -> Result<()> {
    match format {
        Format::Json => {
            serde_json::to_writer_pretty(&mut writer, values).context("serialize JSON export")?;
            writer.write_all(b"\n").context("finish JSON export")?;
        }
        Format::Xml => xml(
            &DevicesXml {
                devices: values.iter().map(DeviceRow::from).collect(),
            },
            &mut writer,
        )?,
        Format::Csv => {
            csv_rows(values.iter().map(DeviceRow::from), writer, DEVICE_HEADERS)?;
        }
    }
    Ok(())
}

pub fn json<W: Write, T: Serialize + ?Sized>(value: &T, mut writer: W) -> Result<()> {
    serde_json::to_writer_pretty(&mut writer, value).context("serialize JSON export")?;
    writer.write_all(b"\n").context("finish JSON export")
}

pub fn history<W: Write>(values: &[TimelineEvent], format: Format, mut writer: W) -> Result<()> {
    match format {
        Format::Json => json(
            &values.iter().map(EventRow::from).collect::<Vec<_>>(),
            &mut writer,
        ),
        Format::Xml => xml(
            &HistoryXml {
                events: values.iter().map(EventRow::from).collect(),
            },
            &mut writer,
        ),
        Format::Csv => csv_rows(values.iter().map(EventRow::from), writer, EVENT_HEADERS),
    }
}

pub fn alerts<W: Write>(values: &[Alert], format: Format, mut writer: W) -> Result<()> {
    match format {
        Format::Json => json(
            &values.iter().map(AlertRow::from).collect::<Vec<_>>(),
            &mut writer,
        ),
        Format::Xml => xml(
            &AlertsXml {
                alerts: values.iter().map(AlertRow::from).collect(),
            },
            &mut writer,
        ),
        Format::Csv => csv_rows(values.iter().map(AlertRow::from), writer, ALERT_HEADERS),
    }
}

pub fn baseline<W: Write>(value: &Baseline, format: Format, mut writer: W) -> Result<()> {
    match format {
        Format::Json => json(value, &mut writer),
        Format::Xml => xml(&BaselineXml::from(value), &mut writer),
        Format::Csv => csv_rows(
            value
                .devices
                .iter()
                .map(|device| BaselineDeviceRow::new(value, device)),
            writer,
            BASELINE_HEADERS,
        ),
    }
}

pub fn comparison<W: Write>(value: &BaselineDiff, format: Format, mut writer: W) -> Result<()> {
    match format {
        Format::Json => json(value, &mut writer),
        Format::Xml => xml(
            &ComparisonXml {
                changes: comparison_rows(value),
            },
            &mut writer,
        ),
        Format::Csv => csv_rows(comparison_rows(value), writer, COMPARISON_HEADERS),
    }
}

pub fn device_table<W: Write>(values: &[Device], mut writer: W) -> Result<()> {
    writeln!(
        writer,
        "{:<28} {:<15} {:<10} LAST SEEN",
        "DEVICE", "ADDRESS", "STATUS"
    )?;
    for device in values {
        writeln!(
            writer,
            "{:<28} {:<15} {:<10} {}",
            truncate(&device.display_name(), 28),
            device.ipv4,
            device.status,
            device.last_seen.format("%Y-%m-%d %H:%M:%S UTC")
        )?;
    }
    Ok(())
}

pub fn scan_table<W: Write>(values: &[Device], mut writer: W) -> Result<()> {
    writeln!(
        writer,
        "{:<3} {:<28} {:<15} {:<12} SERVICES",
        "", "DEVICE", "ADDRESS", "TYPE"
    )?;
    for device in values {
        let services = device
            .services
            .values()
            .map(|service| service.name.clone())
            .collect::<Vec<_>>()
            .join(",");
        writeln!(
            writer,
            "{:<3} {:<28} {:<15} {:<12} {}",
            "+",
            truncate(&device.display_name(), 28),
            device.ipv4,
            device.device_type,
            services
        )?;
    }
    writeln!(
        writer,
        "\n{} device(s) with positive discovery evidence",
        values.len()
    )?;
    Ok(())
}

pub fn history_table<W: Write>(values: &[TimelineEvent], mut writer: W) -> Result<()> {
    if values.is_empty() {
        writeln!(
            writer,
            "No network history yet. Changes appear after discovery scans."
        )?;
    }
    for event in values {
        writeln!(
            writer,
            "{}  {:<9} {}",
            event.occurred_at.format("%Y-%m-%d %H:%M:%S"),
            event.kind.label(),
            event.summary
        )?;
        if !event.detail.is_empty() {
            writeln!(writer, "                     {}", event.detail)?;
        }
    }
    Ok(())
}

pub fn alerts_table<W: Write>(values: &[Alert], mut writer: W) -> Result<()> {
    writeln!(
        writer,
        "{:<20} {:<10} {:<13} SUMMARY",
        "CREATED", "SEVERITY", "STATE"
    )?;
    for alert in values {
        let state = if alert.resolved_at.is_some() {
            "resolved"
        } else if alert.acknowledged {
            "acknowledged"
        } else {
            "open"
        };
        writeln!(
            writer,
            "{:<20} {:<10} {:<13} {}",
            alert.created_at.format("%Y-%m-%d %H:%M:%S"),
            severity_key(alert.severity),
            state,
            alert.summary
        )?;
    }
    Ok(())
}

pub fn comparison_table<W: Write>(value: &BaselineDiff, mut writer: W) -> Result<()> {
    writeln!(
        writer,
        "{} unknown · {} missing · {} changed",
        value.unknown.len(),
        value.missing.len(),
        value.changed.len()
    )?;
    for row in comparison_rows(value) {
        writeln!(
            writer,
            "{:<9} {:<28} {:<15} {}",
            row.change.to_ascii_uppercase(),
            truncate(&row.name, 28),
            row.ip,
            row.details
        )?;
    }
    Ok(())
}

fn xml<W: Write, T: Serialize>(value: &T, mut writer: W) -> Result<()> {
    let mut body = String::new();
    let mut serializer = Serializer::new(&mut body);
    serializer.indent(' ', 2);
    value
        .serialize(serializer)
        .context("serialize XML export")?;
    writer
        .write_all(b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n")
        .context("write XML declaration")?;
    writer
        .write_all(body.as_bytes())
        .context("write XML export")?;
    writer.write_all(b"\n").context("finish XML export")
}

fn csv_rows<W, I, T>(rows: I, writer: W, headers: &[&str]) -> Result<()>
where
    W: Write,
    I: IntoIterator<Item = T>,
    T: Serialize,
{
    let mut csv = csv::WriterBuilder::new()
        .has_headers(false)
        .from_writer(writer);
    csv.write_record(headers).context("write CSV header")?;
    for row in rows {
        csv.serialize(row).context("serialize CSV row")?;
    }
    csv.flush().context("finish CSV export")
}

const DEVICE_HEADERS: &[&str] = &[
    "id",
    "name",
    "status",
    "device_type",
    "confidence",
    "inferred_model",
    "platform",
    "identity_source",
    "ipv4",
    "mac",
    "hostname",
    "vendor",
    "latency_ms",
    "services",
    "sources",
    "first_seen",
    "last_seen",
    "tags",
    "notes",
];
const EVENT_HEADERS: &[&str] = &[
    "id",
    "device_id",
    "occurred_at",
    "kind",
    "severity",
    "summary",
    "detail",
];
const ALERT_HEADERS: &[&str] = &[
    "id",
    "device_id",
    "created_at",
    "resolved_at",
    "severity",
    "rule",
    "summary",
    "detail",
    "acknowledged",
    "state",
];
const BASELINE_HEADERS: &[&str] = &[
    "baseline_id",
    "baseline_name",
    "baseline_created_at",
    "subnet",
    "stable_key",
    "name",
    "ip",
    "mac",
    "services",
];
const COMPARISON_HEADERS: &[&str] = &[
    "change",
    "name",
    "ip",
    "previous_ip",
    "mac",
    "services",
    "details",
];

#[derive(Debug, Serialize)]
#[serde(rename = "devices")]
struct DevicesXml {
    #[serde(rename = "device")]
    devices: Vec<DeviceRow>,
}

#[derive(Debug, Serialize)]
#[serde(rename = "history")]
struct HistoryXml {
    #[serde(rename = "event")]
    events: Vec<EventRow>,
}

#[derive(Debug, Serialize)]
struct EventRow {
    id: String,
    device_id: String,
    occurred_at: String,
    kind: String,
    severity: String,
    summary: String,
    detail: String,
}

impl From<&TimelineEvent> for EventRow {
    fn from(event: &TimelineEvent) -> Self {
        Self {
            id: event.id.to_string(),
            device_id: event
                .device_id
                .map_or_else(String::new, |id| id.to_string()),
            occurred_at: event.occurred_at.to_rfc3339(),
            kind: event_kind_key(event.kind).into(),
            severity: severity_key(event.severity).into(),
            summary: event.summary.clone(),
            detail: event.detail.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename = "alerts")]
struct AlertsXml {
    #[serde(rename = "alert")]
    alerts: Vec<AlertRow>,
}

#[derive(Debug, Serialize)]
struct AlertRow {
    id: String,
    device_id: String,
    created_at: String,
    resolved_at: String,
    severity: String,
    rule: String,
    summary: String,
    detail: String,
    acknowledged: bool,
    state: String,
}

impl From<&Alert> for AlertRow {
    fn from(alert: &Alert) -> Self {
        Self {
            id: alert.id.to_string(),
            device_id: alert
                .device_id
                .map_or_else(String::new, |id| id.to_string()),
            created_at: alert.created_at.to_rfc3339(),
            resolved_at: alert
                .resolved_at
                .map_or_else(String::new, |at| at.to_rfc3339()),
            severity: severity_key(alert.severity).into(),
            rule: alert.rule.clone(),
            summary: alert.summary.clone(),
            detail: alert.detail.clone(),
            acknowledged: alert.acknowledged,
            state: if alert.resolved_at.is_some() {
                "resolved"
            } else if alert.acknowledged {
                "acknowledged"
            } else {
                "open"
            }
            .into(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename = "baseline")]
struct BaselineXml {
    id: String,
    name: String,
    created_at: String,
    subnet: String,
    #[serde(rename = "device")]
    devices: Vec<SnapshotRow>,
}

impl From<&Baseline> for BaselineXml {
    fn from(baseline: &Baseline) -> Self {
        Self {
            id: baseline.id.to_string(),
            name: baseline.name.clone(),
            created_at: baseline.created_at.to_rfc3339(),
            subnet: baseline.subnet.clone(),
            devices: baseline.devices.iter().map(SnapshotRow::from).collect(),
        }
    }
}

#[derive(Debug, Serialize)]
struct SnapshotRow {
    stable_key: String,
    name: String,
    ip: String,
    mac: String,
    services: String,
}

impl From<&DeviceSnapshot> for SnapshotRow {
    fn from(device: &DeviceSnapshot) -> Self {
        Self {
            stable_key: device.stable_key.clone(),
            name: device.name.clone(),
            ip: device.ip.clone(),
            mac: device.mac.clone().unwrap_or_default(),
            services: device
                .services
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join(";"),
        }
    }
}

#[derive(Debug, Serialize)]
struct BaselineDeviceRow {
    baseline_id: String,
    baseline_name: String,
    baseline_created_at: String,
    subnet: String,
    stable_key: String,
    name: String,
    ip: String,
    mac: String,
    services: String,
}

impl BaselineDeviceRow {
    fn new(baseline: &Baseline, device: &DeviceSnapshot) -> Self {
        let snapshot = SnapshotRow::from(device);
        Self {
            baseline_id: baseline.id.to_string(),
            baseline_name: baseline.name.clone(),
            baseline_created_at: baseline.created_at.to_rfc3339(),
            subnet: baseline.subnet.clone(),
            stable_key: snapshot.stable_key,
            name: snapshot.name,
            ip: snapshot.ip,
            mac: snapshot.mac,
            services: snapshot.services,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename = "comparison")]
struct ComparisonXml {
    #[serde(rename = "change")]
    changes: Vec<ComparisonRow>,
}

#[derive(Debug, Serialize)]
struct ComparisonRow {
    change: String,
    name: String,
    ip: String,
    previous_ip: String,
    mac: String,
    services: String,
    details: String,
}

fn comparison_rows(diff: &BaselineDiff) -> Vec<ComparisonRow> {
    let mut rows = Vec::with_capacity(diff.unknown.len() + diff.missing.len() + diff.changed.len());
    rows.extend(
        diff.unknown
            .iter()
            .map(|device| comparison_snapshot_row("unknown", device)),
    );
    rows.extend(
        diff.missing
            .iter()
            .map(|device| comparison_snapshot_row("missing", device)),
    );
    rows.extend(diff.changed.iter().map(|difference| {
        ComparisonRow {
            change: "changed".into(),
            name: difference.after.name.clone(),
            ip: difference.after.ip.clone(),
            previous_ip: difference.before.ip.clone(),
            mac: difference.after.mac.clone().unwrap_or_default(),
            services: difference
                .after
                .services
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join(";"),
            details: difference.changes.join("; "),
        }
    }));
    rows
}

fn comparison_snapshot_row(change: &str, device: &DeviceSnapshot) -> ComparisonRow {
    ComparisonRow {
        change: change.into(),
        name: device.name.clone(),
        ip: device.ip.clone(),
        previous_ip: String::new(),
        mac: device.mac.clone().unwrap_or_default(),
        services: device
            .services
            .iter()
            .map(u16::to_string)
            .collect::<Vec<_>>()
            .join(";"),
        details: String::new(),
    }
}

const fn event_kind_key(kind: EventKind) -> &'static str {
    match kind {
        EventKind::DeviceNew => "device_new",
        EventKind::DeviceOnline => "device_online",
        EventKind::DeviceOffline => "device_offline",
        EventKind::IdentityChanged => "identity_changed",
        EventKind::AddressChanged => "address_changed",
        EventKind::ServicesChanged => "services_changed",
        EventKind::UserMetadataChanged => "user_metadata_changed",
        EventKind::ScanFailed => "scan_failed",
        EventKind::BaselineDifference => "baseline_difference",
    }
}

const fn severity_key(severity: Severity) -> &'static str {
    match severity {
        Severity::Info => "info",
        Severity::Warning => "warning",
        Severity::Critical => "critical",
    }
}

fn truncate(value: &str, width: usize) -> String {
    let mut chars = value.chars();
    let mut result = chars.by_ref().take(width).collect::<String>();
    if chars.next().is_some() && width > 1 {
        result.pop();
        result.push('…');
    }
    result
}

#[derive(Debug, Serialize)]
struct DeviceRow {
    id: String,
    name: String,
    status: String,
    device_type: String,
    confidence: u8,
    inferred_model: String,
    platform: String,
    identity_source: String,
    ipv4: String,
    mac: String,
    hostname: String,
    vendor: String,
    latency_ms: String,
    services: String,
    sources: String,
    first_seen: String,
    last_seen: String,
    tags: String,
    notes: String,
}

impl From<&Device> for DeviceRow {
    fn from(device: &Device) -> Self {
        Self {
            id: device.id.to_string(),
            name: device.display_name(),
            status: device.status.to_string(),
            device_type: device.device_type.to_string(),
            inferred_model: device.inferred_model.clone().unwrap_or_default(),
            platform: device.platform.clone().unwrap_or_default(),
            identity_source: if device.identity_is_user_confirmed() {
                "user_confirmed".into()
            } else {
                "automatic".into()
            },
            confidence: device.confidence,
            ipv4: device.ipv4.to_string(),
            mac: device.mac.clone().unwrap_or_default(),
            hostname: device.hostname.clone().unwrap_or_default(),
            vendor: device.vendor.clone().unwrap_or_default(),
            latency_ms: device
                .latency_ms
                .map_or_else(String::new, |value| value.to_string()),
            services: device
                .services
                .values()
                .map(|service| format!("{}/{}", service.port, service.name))
                .collect::<Vec<_>>()
                .join(";"),
            sources: device
                .sources
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(";"),
            first_seen: device.first_seen.to_rfc3339(),
            last_seen: device.last_seen.to_rfc3339(),
            tags: device
                .user
                .tags
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(";"),
            notes: device.user.notes.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use uuid::Uuid;

    use crate::{app::demo_state, config::Config};

    use super::*;

    #[test]
    fn device_exports_include_identity_in_every_machine_format() {
        let state = demo_state(&Config::default());
        let mut json_output = Vec::new();
        devices(&state.devices, Format::Json, &mut json_output).expect("json");
        assert!(
            String::from_utf8(json_output)
                .expect("utf8")
                .contains("Home NAS")
        );

        let mut csv_output = Vec::new();
        devices(&state.devices, Format::Csv, &mut csv_output).expect("csv");
        let csv = String::from_utf8(csv_output).expect("utf8");
        assert!(csv.contains(
            "name,status,device_type,confidence,inferred_model,platform,identity_source"
        ));
        assert!(csv.contains("Home NAS"));

        let mut xml_output = Vec::new();
        devices(&state.devices, Format::Xml, &mut xml_output).expect("xml");
        let xml = String::from_utf8(xml_output).expect("utf8");
        assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
        assert!(xml.contains("<devices>"));
        assert!(xml.contains("<device>"));
        assert!(xml.contains("<name>Home NAS</name>"));
    }

    #[test]
    fn empty_csv_exports_still_include_stable_headers() {
        let mut devices_output = Vec::new();
        devices(&[], Format::Csv, &mut devices_output).expect("empty devices");
        let csv = String::from_utf8(devices_output).expect("utf8");
        assert!(csv.starts_with("id,name,status,device_type"));

        let mut history_output = Vec::new();
        history(&[], Format::Csv, &mut history_output).expect("empty history");
        let csv = String::from_utf8(history_output).expect("utf8");
        assert!(csv.starts_with("id,device_id,occurred_at,kind,severity"));
    }

    #[test]
    fn history_and_alerts_have_json_xml_and_csv_serializers() {
        let state = demo_state(&Config::default());
        let events = state.events.iter().cloned().collect::<Vec<_>>();

        for format in [Format::Json, Format::Xml, Format::Csv] {
            let mut output = Vec::new();
            history(&events, format, &mut output).expect("history export");
            assert!(!output.is_empty());

            let mut output = Vec::new();
            alerts(&state.alerts, format, &mut output).expect("alert export");
            assert!(!output.is_empty());
        }
    }

    #[test]
    fn baseline_and_comparison_support_every_export_format() {
        let state = demo_state(&Config::default());
        let snapshots = state
            .devices
            .iter()
            .map(DeviceSnapshot::from)
            .collect::<Vec<_>>();
        let known = Baseline {
            id: Uuid::new_v4(),
            name: "Office & lab".into(),
            created_at: Utc::now(),
            subnet: "192.168.1.0/24".into(),
            devices: snapshots.clone(),
        };
        let diff = BaselineDiff {
            unknown: snapshots.first().cloned().into_iter().collect(),
            missing: snapshots.get(1).cloned().into_iter().collect(),
            changed: Vec::new(),
        };

        for format in [Format::Json, Format::Xml, Format::Csv] {
            let mut output = Vec::new();
            baseline(&known, format, &mut output).expect("baseline export");
            assert!(!output.is_empty());

            let mut output = Vec::new();
            comparison(&diff, format, &mut output).expect("comparison export");
            assert!(!output.is_empty());
        }

        let mut xml = Vec::new();
        baseline(&known, Format::Xml, &mut xml).expect("baseline XML");
        let xml = String::from_utf8(xml).expect("UTF-8 XML");
        assert!(xml.contains("<baseline>"));
        assert!(xml.contains("<name>Office &amp; lab</name>"));
    }
}
