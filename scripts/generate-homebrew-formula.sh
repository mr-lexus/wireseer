#!/bin/sh
set -eu

usage() {
  echo "usage: $0 VERSION OWNER/REPOSITORY SOURCE_ARCHIVE OUTPUT" >&2
  exit 2
}

[ "$#" -eq 4 ] || usage

version=$1
repository=$2
source_archive=$3
output=$4

case "$version" in
  ''|*[!0-9.]*|.*|*.)
    echo "invalid stable version: $version" >&2
    exit 2
    ;;
esac

case "$repository" in
  */*) ;;
  *)
    echo "repository must be OWNER/REPOSITORY" >&2
    exit 2
    ;;
esac

[ -f "$source_archive" ] || {
  echo "source archive not found: $source_archive" >&2
  exit 2
}

if command -v sha256sum >/dev/null 2>&1; then
  sha256=$(sha256sum "$source_archive" | awk '{print $1}')
else
  sha256=$(shasum -a 256 "$source_archive" | awk '{print $1}')
fi

homepage="https://github.com/$repository"
source_url="$homepage/releases/download/v$version/wireseer-$version.tar.gz"

sed \
  -e "s|@HOMEPAGE@|$homepage|g" \
  -e "s|@SOURCE_URL@|$source_url|g" \
  -e "s|@SHA256@|$sha256|g" \
  packaging/homebrew/wireseer.rb.in >"$output"

echo "Generated $output"
echo "Source SHA-256: $sha256"
