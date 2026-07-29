# Wireseer product and architecture plan

Wireseer is a local-first, read-only network inventory for networks the operator owns or
is authorized to inspect. It never uploads discovered identifiers and it limits active
discovery to conservative, cancellable connect checks.

## Architecture

```text
CLI/config ──────┐
                 v
interface selection -> scan coordinator -> bounded providers
                                          (local, TCP, DNS; optional protocols)
                                                |
                                                v
                                         Observation stream
                                                |
                                                v
TUI input -> Commands -> Reducer -> device merge -> fingerprint/confidence
                          |              |                 |
                          |              +-------> alerts/diffs
                          |                                |
                          +----------> SQLite writer <-----+
                          |
                          +----------> RenderSnapshot -> ratatui
```

The reducer is the single owner of mutable application state. Workers communicate through
bounded Tokio channels and never render. The SQLite writer is serialized and flushed on
shutdown. Slow or unsupported providers report health independently; they cannot block the
scan or erase stronger facts. Observed facts, heuristic inferences, and user metadata remain
separate in the domain model.

Crate modules:

- `app`: state, actions, reducer, keyboard commands, task coordination.
- `network`: platform-neutral interface model and cancellable discovery providers.
- `devices`: observations, merged records, services, fingerprint evidence.
- `history`: migrations, persistence, scan/device diffs, baselines.
- `tui`: semantic themes, breakpoints, screens, widgets, dialogs, input.
- `config`, `cli`, `alerts`, `export`, `logging`: product boundaries.

## Visual design system

The signature is a small signal path rather than a mascot or shield:

```text
WIRESEER  ●──┬────●──╼
          local signal intelligence
```

`WIRE` is primary text, `SEER` and the live trace use the accent, and the branch suggests that one
physical network exposes many independently observed devices. The mark uses ordinary Unicode and
has the ASCII fallback `o--+----o-->`; it never depends on Nerd Font glyphs. The visual tone is
instrument-grade rather than theatrical: graphite surfaces, cold cyan signal, restrained violet
telemetry, compact uppercase labels, and no Matrix green, fake shell prompts, or ornamental code.

Wireseer uses an unboxed page frame with a slim identity header, content surfaces separated
by spacing or a single divider, and one contextual action bar. Focus is shown with an accent
rule and selection background, not a maze of borders. Metric values reserve fixed widths so
live updates do not shift neighboring content.

Semantic theme tokens are `background`, `surface`, `surface_active`, `border`,
`border_focused`, `text_primary`, `text_secondary`, `text_muted`, `accent`, `success`,
`warning`, `danger`, `info`, `selection`, `selection_text`, `chart_primary`, and
`chart_secondary`. Built-ins are Wireseer Dark, Catppuccin Mocha/Latte, Dracula, Nord, Midnight
Blue, Acid, Paper Light, High Contrast, Monochrome, and Color-Blind Friendly. Status always has a
word or glyph shape in addition to color.

Typography roles:

- page title: bold primary text;
- eyebrow/section label: uppercase muted text;
- important metric: bold semantic color, stable width;
- body: primary text; secondary metadata is muted;
- key hint: accent key plus secondary label;
- inferred values: `~` prefix; user metadata: pencil marker/fallback `user:`;
- observed evidence: check/fallback `+`; unavailable evidence: dash/fallback `-`.

Icons are selected by an `IconSet` (`Nerd`, `Unicode`, `Ascii`). Every icon is paired with a
readable label and ASCII mode removes all non-ASCII chart/block characters. Animation runs
only while work or a transient highlight is active, defaults to 8 fps at most, and can be
disabled.

## Responsive layout

| Class | Bounds | Behavior |
| --- | --- | --- |
| Wide | width >= 140 | Three dashboard cards; devices list plus full inspector; rich columns |
| Standard | 90..=139 | Two dashboard columns; compact inspector; reduced columns |
| Compact | 70..=89 | One content panel; inspector and dialogs become pages; identity columns |
| Too small | width < 70 or height < 18 | Centered safe-size message; input and quit still work |

Columns disappear in this order: tags, confidence, services, vendor, latency, MAC, hostname.
Status, display name, and IP survive to compact mode. Long identity values receive remaining
space before secondary metadata.

## Screen mockups

All devices and events below are fictional. Addresses use the IETF TEST-NET-1 documentation range
`192.0.2.0/24`; release screenshots are rendered from the same network-free demo state.

### Dashboard — wide

```text
 WIRESEER  ●──┬────●──╼  Demo network · demo0 · 192.0.2.0/24  WATCH  ● ACTIVE
 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
 NETWORK                    DEVICE ACTIVITY                   HEALTH
  24  online   3 offline    ▁▂▂▃▅▆▅▇▆▇█  +2 this hour        Gateway    ONLINE
   2  unknown  4 changed    Discovery: TCP · DNS · mDNS      Queue          14
  Scan  ███████████░ 76%    Routers 1 · Computers 8          Warnings        2

 RECENT EVENTS                                      COMMON SERVICES
  12:42  NEW      Samsung TV             .48        HTTPS 12  SSH 7  SMB 3
  12:39  CHANGED  Home NAS · HTTPS added  .20        VENDORS  Apple 7 · Dell 4
  12:35  OFFLINE  Office printer          .70
 ─────────────────────────────────────────────────────────────────────────────
  1 Dashboard  2 Devices  3 History  4 Compare  5 Alerts  6 Logs  7 Settings
  q Quit  ? Help  Ctrl+P Commands  r Scan
```

### Devices / inspector — wide and standard

```text
 WIRESEER  ●──┬────●──╼ / DEVICES        6 total · 4 online      NORMAL  ● SCANNING
 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
   / Search by name, IP, vendor, service…
 DEVICES                                      HOME NAS                         
 ● [NAS] Home NAS       192.0.2.20   2ms    Synology DS923+        ~ NAS 96%
 ● [PC]  Desktop        192.0.2.25  <1ms    IP       192.0.2.20
 ● [TV]  Living room    192.0.2.48  12ms    MAC      02:00:00:00:00:30
 ○ [P]   Office printer 192.0.2.70     —     Vendor   Synology
 ? [?]   Unknown        192.0.2.91   7ms
                                              SERVICES
                                              SSH  HTTP  HTTPS  SMB
                                              EVIDENCE
                                              + Vendor match             +35
                                              + DiskStation HTTP title   +15
 ─────────────────────────────────────────────────────────────────────────────
  6 devices  Sort status · Filter all     ↵ Details  Space Select  / Search
```

At narrow layout widths, the selected device opens as its own page and secondary columns are
removed before status, identity, or IP. Independently, Row density controls the inventory rows:
Detailed uses two meaningful lines of facts; Compact keeps the primary facts on one line.

### Device details

```text
 < DEVICES / HOME NAS                        ONLINE · seen now · confidence 96%
 OVERVIEW              ADDRESSES                 SERVICES
 ~ NAS                 IPv4 192.0.2.20           22 SSH       open · 2ms
 Synology DS923+       MAC  00:11:22:33:44:55    443 HTTPS    open · 3ms
 user: Home NAS        Previous .18              445 SMB      observed mDNS

 DISCOVERY EVIDENCE                              TIMELINE
 + Synology OUI matches NAS family          +35  12:39 HTTPS service added
 + ports 5000/5001 observed                 +20  Jul 19 IP .18 -> .20
 + mDNS advertises file sharing             +18  Jul 10 first observed
 ─────────────────────────────────────────────────────────────────────────────
  Tab Section  e Edit  t Tags  n Notes  h History  Esc Back
```

### History

```text
 WIRESEER  ●──┬────●──╼ / HISTORY      Filter: all · all devices · today
 TODAY
  12:42  + NEW DEVICE       Samsung TV                       192.0.2.48
          First observed by TCP and mDNS
  12:39  ~ SERVICES CHANGED Home NAS                         HTTPS added
  12:35  - OFFLINE          Office Printer                   seen 8m ago
 YESTERDAY
  22:14  i NAME CHANGED     phone-4f2a -> Mira's Phone
 ─────────────────────────────────────────────────────────────────────────────
  j/k Navigate  f Filter  Enter Device  e Export  ? Help
```

### Compare / baseline

```text
 WIRESEER  ●──┬────●──╼ / COMPARE      Current scan vs 12:30 · 3 differences
 ADDED                         REMOVED                       CHANGED
 + Samsung TV .48             - Old laptop .32             ~ Home NAS
                                                             + HTTPS 443
                                                             - HTTP 80
                                                             IP .18 -> .20
 BASELINE ATTENTION
 ! Unknown device: Samsung TV       ! Expected printer is missing
 ─────────────────────────────────────────────────────────────────────────────
  b Compare baseline  p Previous scan  Enter Inspect  e Export
```

### Alerts

```text
 WIRESEER  ●──┬────●──╼ / ALERTS       2 active · 4 resolved
 ACTIVE
 ! WARNING  Unknown device appeared                         4 minutes ago
            Samsung TV · 192.0.2.48 · first seen today
 x CRITICAL Gateway MAC changed                            12 minutes ago
            Review the old and new observed addresses
 RESOLVED
 i INFO     Office printer returned                         yesterday
 ─────────────────────────────────────────────────────────────────────────────
  Enter Inspect  a Acknowledge  f Filter  Tab Active/Resolved
```

### Logs

```text
 WIRESEER  ●──┬────●──╼ / LOGS         level >= info · provider all · follow on
 12:42:03 INFO  scan       device observed       ip=192.0.2.48 source=tcp
 12:42:03 DEBUG dns        reverse lookup timed out ip=192.0.2.51
 12:42:04 WARN  mdns       provider unavailable  reason=permission denied
 ─────────────────────────────────────────────────────────────────────────────
  / Search  l Level  p Provider  f Follow  c Clear view
```

### Settings

```text
 WIRESEER  ●──┬────●──╼ / SETTINGS
 APPEARANCE                         DISCOVERY
 > Theme          Wireseer Dark      Scan mode       Normal
   Icons          Unicode           Refresh         30 s
   Animation      On                TCP checks      On
   Row density    Detailed          mDNS            On (available)
                                    ARP             Unavailable — see doctor
 DATA
   Retention      90 days           Vendor DB       not installed
 ─────────────────────────────────────────────────────────────────────────────
  j/k Navigate  Enter Change  s Save  r Reset field  ? Help
```

### Help and command palette overlays

Help maps every numbered workspace to its purpose and shows only the shortcuts implemented by the
current screen. The command palette contains every workspace plus scan, view, appearance, export,
diagnostic, help, and quit actions. Its list scrolls with the selection and reports the visible
range and total count, so commands never disappear below the dialog edge.

```text
             COMMANDS                         HELP · DEVICES
        > Open Dashboard                  SCREENS              THIS SCREEN
          Open Devices                    1 Dashboard          ↑/k Previous device
          Open History                    2 Devices            ↓/j Next device
          Open Compare                    3 History            Enter Details
          Open Alerts                     4 Compare            Space Select
          Open Logs                       5 Alerts             / Search
          Open Settings                   6 Logs               f Filter
          Rescan                          7 Settings           s Sort
          ...
        ↑↓ Choose · Enter Run · 1–9/23    Ctrl+P Commands · ?/Esc Close
```

### Empty, loading, and error states

```text
 NO DEVICES DISCOVERED YET        SCANNING 192.0.2.0/24         TCP UNAVAILABLE
 The scan is still running.       Hosts     ███████░ 58%        Permission denied.
 Check the interface or press r.  Enriching ███░░░░ 21%        DNS will continue.
                                  Found 12 · active 18          Run `wireseer doctor`.
```

## Core contracts

`Observation` contains source, timestamp, interface, address, optional latency, and typed
facts. `Device` contains stable identity, accumulated `Observed<T>` facts with provenance,
`Inference<T>` with confidence/evidence, and `UserMetadata`. `Service` is normalized by
transport and port. Lower-confidence evidence can add provenance but cannot replace a
stronger fact.

```text
DiscoveryProvider::scan(ScanContext, ObservationSender, CancellationToken)
AppEvent = Input | Tick | Observation | ScanProgress | ProviderHealth | DbResult | Shutdown
Action   = Navigate | ChangeView | StartScan | CancelScan | ApplyFilter | OpenOverlay | ...
```

Provider requirements: bounded concurrency, per-attempt timeout, cancellation at every wait,
incremental observations, structured progress, independent failure, and no authentication or
payload probing. Online status is positive-evidence based: lack of ICMP alone never means
offline.

## SQLite schema

Migrations create:

- `devices(id, stable_key, first_seen, last_seen, last_changed, user_name, inferred_type,
  confidence, tags_json, notes)`;
- `addresses(id, device_id, kind, address, mac, first_seen, last_seen, is_current)`;
- `observations(id, scan_id, device_id, observed_at, source, payload_json)`;
- `services(id, device_id, transport, port, name, metadata_json, first_seen, last_seen,
  is_current)`;
- `scans(id, started_at, finished_at, interface, subnet, mode, status, summary_json)`;
- `events(id, device_id, scan_id, occurred_at, kind, severity, summary, details_json)`;
- `baselines(id, name, created_at, subnet, snapshot_json, is_active)`;
- `alerts(id, device_id, event_id, created_at, resolved_at, severity, rule, summary)`.

Foreign keys are enabled; indexes cover stable keys, timestamps, device timelines, current
addresses, and unresolved alerts. Schema changes are numbered and tested against an empty and
previous database.

## Keyboard model

Global: `1` dashboard, `2` devices, `3` history, `4` compare, `5` alerts, `6` logs,
`7` settings, `Ctrl+P` palette, `?` contextual help, `r` scan, `x` cancel an active scan,
`q` quit (unless editing), and `Esc` closes the topmost layer. The persistent footer always
renders quit, help, command search, and scan/cancel before optional contextual shortcuts.
Lists use `j/k`, arrows, `g/G`, PageUp/PageDown; `Enter` drills in; `Space` toggles selection.
`/` searches, `f` filters, `s` sorts, and `e` exports only where applicable. Dialogs trap
input and never leak keys to the page.
Mouse input maps to the same actions: footer clicks switch screens, row clicks select or activate,
the wheel moves through lists and menus, middle-click toggles device selection, and right-click
acts like `Esc`. Enabling or disabling mouse input from Settings takes effect immediately.

## Phased checklist

- [x] Foundation: CLI/config/logging, device model, reducer, semantic themes, responsive shell,
  mock provider, dashboard/devices/details, palette/help, empty/loading/error variants.
- [x] Discovery: interfaces/subnets, local host/gateway hints, bounded TCP checks, reverse DNS,
  progress/cancellation; Linux neighbor cache works and unsupported raw ICMP/platform ARP paths
  degrade explicitly.
- [x] Enrichment: offline vendor DB import/lookup, mDNS/SSDP, optional bounded HTTP metadata, and
  explainable fingerprints. NetBIOS and TLS certificate adapters retain explicit degraded status.
- [x] Persistence: versioned migrations, inventory/history/diffs, timelines, user metadata,
  baselines, alerts, and retention pruning.
- [x] Polish: responsive filters/search/sort, alerts, JSON/CSV, settings, doctor, themes, compact mode,
  docs, fixtures, snapshot/layout/input/migration/integration tests, CI, screenshots.

Every phase exits with a runnable binary and must pass `cargo fmt --check`, `cargo test`,
`cargo clippy --all-targets -- -D warnings`, and `cargo build` before the next phase is closed.
