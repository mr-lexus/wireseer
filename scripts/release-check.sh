#!/bin/sh
set -eu

version=${1:-}
repository=${2:-}

[ -n "$version" ] || {
  echo "usage: $0 VERSION OWNER/REPOSITORY" >&2
  exit 2
}

[ -n "$repository" ] || {
  echo "usage: $0 VERSION OWNER/REPOSITORY" >&2
  exit 2
}

manifest_version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)
[ "$manifest_version" = "$version" ] || {
  echo "tag version $version does not match Cargo.toml version $manifest_version" >&2
  exit 1
}

expected_repository="https://github.com/$repository"
manifest_repository=$(sed -n 's/^repository = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)
[ "$manifest_repository" = "$expected_repository" ] || {
  echo "Cargo.toml repository must be $expected_repository (found $manifest_repository)" >&2
  exit 1
}

placeholder_report=$(mktemp)
trap 'rm -f "$placeholder_report"' EXIT
if grep -En 'github\.com/example/|<repository-url>|OWNER/REPOSITORY|OWNER/tap' \
  Cargo.toml README.md CHANGELOG.md SECURITY.md docs/INSTALLATION.md >"$placeholder_report"; then
  echo "release metadata still contains placeholders:" >&2
  sed -n '1,80p' "$placeholder_report" >&2
  exit 1
fi

grep -Eq "^## \[$version\]" CHANGELOG.md || {
  echo "CHANGELOG.md has no release heading for $version" >&2
  exit 1
}

cargo fmt --all -- --check
cargo test --all-targets --locked
cargo clippy --all-targets --locked -- -D warnings
cargo package --locked
