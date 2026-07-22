# Lantern CLI

Lantern's command-line interface is currently documented in English. Run `lantern --help` for
the command catalogue and `lantern <command> --help` for command-specific options and examples.
Running `lantern` without a command opens the interactive TUI.

## Output contract

The `scan`, `devices`, `history`, and `alerts` commands accept:

```text
--format table|json|xml|csv
--output PATH
```

`table` is the human-readable default. JSON, XML, and CSV output contains only the selected
serialization on stdout. Diagnostics and failures go to stderr. A successful command exits with
status 0; invalid arguments and runtime failures return a nonzero status.

Omitting `--output`, or passing `--output -`, writes to stdout. Any other path is created or
replaced. Successful file output is silent, which makes the commands suitable for scripts.
CSV output always includes a header row, even when no records match.

## Read stored data

```bash
lantern devices
lantern devices --online --format json
lantern devices --format xml --output devices.xml

lantern history --limit 500 --format csv --output history.csv
lantern alerts --open --format json | jq '.[] | .summary'
```

Device JSON retains the complete structured inventory model, including services and discovery
evidence. Device XML and CSV use a stable flattened record with identity, address, vendor,
service, source, timestamp, tag, and note fields. History and alert exports use lowercase
machine keys such as `device_online`, `warning`, and `open`.

XML documents are UTF-8 and use these roots:

- `<devices>` containing `<device>` records;
- `<history>` containing `<event>` records;
- `<alerts>` containing `<alert>` records;
- `<baseline>` containing baseline metadata and `<device>` records;
- `<comparison>` containing `<change>` records.

## Scan once

```bash
lantern scan --mode quick
lantern scan --interface eth0 --format json
lantern scan --subnet 192.168.1.0/24 --format xml --output scan.xml
```

A scan emits its final result only after all bounded providers finish. Discovery failures return a
nonzero status and do not emit a successful machine document.

## Export complete stored datasets

`export` supports `devices`, `history`, `alerts`, `baseline`, and `comparison` in JSON, XML, or CSV:

```bash
lantern export --kind devices --format json
lantern export --kind history --format xml --output history.xml
lantern export --kind alerts --format json --output alerts.json
lantern export --kind baseline --format csv --output baseline.csv
lantern export --kind comparison --format json | jq '.changed'
```

Baseline and comparison export require an active baseline. Create one after reviewing the current
inventory:

```bash
lantern baseline create --name "Known network"
```

## Automation examples

```bash
# Count stored devices without parsing a table.
lantern devices --format json | jq 'length'

# Keep only online devices in a JSON artifact.
lantern devices --online --format json --output online-devices.json

# Fail a shell pipeline when Lantern cannot read or serialize its inventory.
set -o pipefail
lantern alerts --open --format json | jq -e 'all(.[]; .severity != "critical")'
```

Lantern never scans unless a scan command or the interactive TUI explicitly starts discovery.
The read and export commands operate on the local SQLite inventory.
