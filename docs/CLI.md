# Wireseer CLI

Wireseer's command-line interface is currently documented in English. Run `wireseer --help` for
the command catalogue and `wireseer <command> --help` for command-specific options and examples.
Running `wireseer` without a command opens the interactive TUI.

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
wireseer devices
wireseer devices --online --format json
wireseer devices --format xml --output devices.xml

wireseer history --limit 500 --format csv --output history.csv
wireseer alerts --open --format json | jq '.[] | .summary'
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
wireseer scan --mode quick
wireseer scan --interface eth0 --format json
wireseer scan --subnet 192.168.1.0/24 --format xml --output scan.xml
```

A scan emits its final result only after all bounded providers finish. Discovery failures return a
nonzero status and do not emit a successful machine document.

## Export complete stored datasets

`export` supports `devices`, `history`, `alerts`, `baseline`, and `comparison` in JSON, XML, or CSV:

```bash
wireseer export --kind devices --format json
wireseer export --kind history --format xml --output history.xml
wireseer export --kind alerts --format json --output alerts.json
wireseer export --kind baseline --format csv --output baseline.csv
wireseer export --kind comparison --format json | jq '.changed'
```

Baseline and comparison export require an active baseline. Create one after reviewing the current
inventory:

```bash
wireseer baseline create --name "Known network"
```

## Automation examples

```bash
# Count stored devices without parsing a table.
wireseer devices --format json | jq 'length'

# Keep only online devices in a JSON artifact.
wireseer devices --online --format json --output online-devices.json

# Fail a shell pipeline when Wireseer cannot read or serialize its inventory.
set -o pipefail
wireseer alerts --open --format json | jq -e 'all(.[]; .severity != "critical")'
```

Wireseer never scans unless a scan command or the interactive TUI explicitly starts discovery.
The read and export commands operate on the local SQLite inventory.
