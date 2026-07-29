# Security policy

## Scope and intended use

Wireseer is for networks the operator owns or is explicitly authorized to inspect. It provides
inventory and change awareness, not exploitation or vulnerability assessment.

Wireseer intentionally excludes credential attacks, authentication attempts, brute force, path
enumeration, stealth scanning, raw SYN scanning, flooding, form submission, arbitrary command
hooks, and destructive actions. Do not propose contributions that add those behaviors.

## Privacy boundary

Discovery data is stored locally in SQLite, TOML, and structured log files. Offline vendor lookup
never sends MAC/IP data to a third party. Export occurs only when the operator explicitly invokes
it. Treat exported inventory as sensitive because hostnames, addresses, and service exposure can
describe a private network.

## Reporting a security issue

Do not open a public issue for a vulnerability that exposes private inventory, bypasses configured
scan limits, or leaves a terminal unusable. Contact the maintainers privately through the security
contact configured for the repository. Include the Wireseer version, operating system, reproduction,
impact, and whether the issue requires an untrusted network response.

## Parser and network hardening

Network text is bounded before parsing. HTTP metadata is lossy-decoded, length-limited, and stored
as passive metadata. SQLite uses bound parameters. Subnet size, concurrency, and timeouts are
validated before a scan. Contributors should add fixtures for malformed/truncated inputs and must
not introduce an unbounded read or redirect loop.

## Release integrity

Stable releases publish `SHA256SUMS` for the source package, generated Homebrew formula, and every
native binary archive. Public GitHub releases also publish build-provenance attestations verifiable
with `gh attestation verify ASSET -R OWNER/REPOSITORY`. Direct-download binaries are not Apple
Developer ID- or Authenticode-signed in version 0.1.0; verify the checksum and provenance before
running them. Homebrew installs build from the immutable, checksummed source archive.
