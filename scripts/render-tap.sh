#!/usr/bin/env bash
# Render the Homebrew cask and formula for a release into a tap checkout.
#
#   scripts/render-tap.sh <version> <SHA256SUMS> <tap dir>
#
# The release workflow runs this against a clone of 2Toad/homebrew-tap.

set -euo pipefail

[[ $# -eq 3 ]] || { echo "usage: $0 <version> <SHA256SUMS> <tap dir>" >&2; exit 2; }
version=$1 sums=$2 tap=$3
src=$(cd "$(dirname "$0")/../packaging/homebrew" && pwd)

sum() {
  local file=$1 hash
  hash=$(awk -v f="$file" '$2 == f { print $1 }' "$sums")
  [[ $hash =~ ^[0-9a-f]{64}$ ]] || { echo "error: no checksum for $file in $sums" >&2; exit 1; }
  echo "$hash"
}

app_sha=$(sum "antfarm-$version-macos-universal.dmg")
cli_sha=$(sum "antfarm-$version-macos-universal.tar.gz")

mkdir -p "$tap/Casks" "$tap/Formula"
for f in Casks/antfarm.rb Formula/antfarm.rb; do
  sed -e "s/@VERSION@/$version/g" -e "s/@APP_SHA256@/$app_sha/g" -e "s/@CLI_SHA256@/$cli_sha/g" \
    "$src/$f" >"$tap/$f"
  if grep -q '@[A-Z0-9_]*@' "$tap/$f"; then
    echo "error: unrendered placeholder left in $f" >&2
    exit 1
  fi
done
cp "$src/README.md" "$tap/README.md"
