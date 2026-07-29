# Wireseer user guide

Wireseer discovers and remembers devices on a local network from a keyboard-driven terminal UI.
It is designed for networks you own or are explicitly authorized to inspect. Observations,
inventory, history, notes, and exports stay on the computer running Wireseer.

## Safe first session

Start by checking the selected interface, storage paths, and provider support without scanning:

```bash
wireseer doctor
```

Then open the interactive UI:

```bash
wireseer
```

Wireseer selects the most likely active IPv4 interface and starts a normal discovery pass. Devices
appear incrementally as providers return evidence. Press `x` to cancel immediately or `q` to quit;
quitting during an active scan asks for confirmation.

If automatic selection chose a VPN, container, or other adapter, exit and specify the intended
interface in the config file or run a one-shot command with `--interface`.

## What discovery does

The default normal mode combines:

- the local IPv4 interface and gateway hints;
- bounded ordinary TCP connect attempts to a conservative fixed port set;
- reverse DNS through the system resolver;
- one mDNS/DNS-SD service-enumeration query;
- one SSDP/UPnP discovery request;
- the Linux kernel's public ARP neighbor cache when available.

It does not authenticate, exploit, brute-force, crawl paths, run external scanning commands, or use
raw SYN scans. A silent device is not considered proof that the device is offline. See
[SECURITY.md](../SECURITY.md) for the exact boundary.

## Workspaces

The footer always shows the numbered workspaces and the most important actions.

| Key | Workspace | Purpose |
| --- | --- | --- |
| `1` | Dashboard | Counts, activity, provider health, recent events, and common services |
| `2` | Devices | Searchable and sortable current inventory |
| `3` | History | Global event timeline |
| `4` | Compare | Previous-scan and named-baseline differences |
| `5` | Alerts | Active and resolved changes requiring attention |
| `6` | Logs | In-app operational log view |
| `7` | Settings | Theme, icons, animation, density, scan mode, and persistence |

Use arrows or `j`/`k` in lists, `Home`/`End` or `g`/`G` for the ends, and `PageUp`/`PageDown` for
larger jumps. `?` opens contextual help. `Ctrl+P` opens a searchable catalogue of every command.
`Esc` closes the topmost dialog or returns from a detail view.

## Find and inspect a device

Press `2`, then `/` and type any part of a device name, hostname, IP, MAC, vendor, service, tag, or
note. Press `Enter` to inspect the selected device. The details view separates:

- observed addresses and services;
- inferred type, product family, platform, confidence, and supporting evidence;
- local user-confirmed identity, tags, and notes;
- timestamps and timeline entries.

An inferred value is marked with `~`; observed evidence is marked with `+` or a check; user metadata
is explicitly marked as local. Automatic inference never overwrites a user-confirmed identity.

In the inventory, `f` cycles status filters, `s` cycles sorts, and `Space` toggles multi-selection.
Detailed rows show provenance and endpoints on a second line; Compact rows keep primary facts on one
line. Narrow terminals hide secondary columns before identity or address.

## Confirm identity and add context

From device details:

- `e` sets an exact local name/model, or clears it to restore automatic detection;
- `t` edits comma-separated local tags;
- `n` edits a local note.

These values are stored in the local SQLite inventory. They are never sent to a lookup service.
Treat exports as sensitive because they can describe internal devices and services.

## Scanning modes

| Mode | Behavior |
| --- | --- |
| Quick | Ten common management/service TCP ports plus enabled supporting providers |
| Normal | Moderate default: 25 TCP ports, bounded concurrency, DNS, mDNS, SSDP, and local facts |
| Deep | Explicit wider fixed set of 37 TCP ports; still bounded and non-stealthy |
| Passive | No TCP connects; enabled local/multicast providers can still send their normal queries |
| Watch | Repeats after the configured delay and only after the previous scan finishes |

Press `r` to start another scan. `x` cancels active provider work. Watch mode can be opened directly:

```bash
wireseer watch --interface en0
```

For automation, a one-shot scan waits for all bounded providers and then emits one result:

```bash
wireseer scan --mode quick
wireseer scan --interface en0 --format json
wireseer scan --subnet 192.168.1.0/24 --format csv --output scan.csv
```

An explicit subnet is rejected before probing when it exceeds `host_limit` (1,024 hosts by
default). Prefer a narrower authorized CIDR instead of raising the limit.

## History, baselines, and alerts

After a completed scan, positive observations update the inventory and timeline. Devices previously
online but absent from the complete observation set can transition to offline. Wireseer records
new devices, address/service/identity changes, online/offline changes, and scan failures.

After reviewing a known-good inventory, create a baseline:

```bash
wireseer baseline create --name "Known home network"
wireseer baseline compare
```

The Compare workspace separates added, missing, and changed devices. A device absent from the
baseline receives a visible marker that does not depend on color. Alerts surface notable changes;
acknowledging an alert changes local workflow state but does not change the underlying observation.

## Export and automation

Human-facing tables are the default. JSON, XML, and CSV modes contain only serialized records on
stdout, while errors go to stderr. This makes them safe to pipe:

```bash
wireseer devices --online --format json | jq 'length'
wireseer history --limit 500 --format csv --output history.csv
wireseer alerts --open --format json
wireseer export --kind comparison --format json --output comparison.json
```

Successful file exports are silent and replace the destination. CSV includes headers even for an
empty result. See [CLI.md](CLI.md) for schemas, XML roots, and more automation examples.

## Appearance and accessibility

Settings includes eleven themes, live theme preview, animation control, two row densities, mouse
input, and three icon modes:

- Unicode is the safe default;
- ASCII removes all non-ASCII icons and charts;
- Nerd Font mode must be selected explicitly after configuring a compatible terminal font.

Meaning never depends only on color or on an icon. High Contrast, Monochrome, and Color-Blind
Friendly themes are included. Every mouse action has a keyboard equivalent.

## Configuration and local files

Use these commands instead of guessing platform paths:

```bash
wireseer config path
wireseer config init
wireseer config show
```

The configuration reference documents every setting and validation rule in
[CONFIGURATION.md](CONFIGURATION.md). The SQLite inventory and structured log live in the data
directory reported by `config path` and `doctor`. Retention pruning runs on startup and preserves
the current inventory and active baseline.

## Offline vendor names

Wireseer never sends MAC addresses to a vendor lookup API. Install the official public IEEE
registries locally:

```bash
wireseer vendor update
wireseer vendor status
```

Or import an IEEE CSV/OUI text file:

```bash
wireseer vendor import oui.csv
```

Randomized/private MAC addresses intentionally have no public vendor result. Hostname and service
evidence can still contribute to a conservative device-type inference.

## If something looks wrong

Run `wireseer doctor` first. Common causes are an automatically selected VPN/container interface,
a local firewall blocking multicast, an intentionally silent device, a subnet over the safety
limit, or a terminal font that does not contain selected Nerd Font glyphs. Provider degradation is
independent: mDNS failure, for example, does not discard TCP, DNS, SSDP, or local observations.
