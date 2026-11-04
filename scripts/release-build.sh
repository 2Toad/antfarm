#!/usr/bin/env bash
# Build, sign, and notarize an antfarm release for macOS.
#
# Produces, in dist-release/:
#   antfarm-<version>-macos-universal.dmg      the desktop app
#   antfarm-<version>-macos-universal.tar.gz   the CLI
#   SHA256SUMS
#
# The release workflow runs this on GitHub Actions, and it runs the same way on a Mac
# for a dry run. See docs/RELEASING.md.
#
# Environment:
#   SIGN_IDENTITY     codesigning identity (default: the 2Toad Developer ID Application)
#   NOTARY_PROFILE    notarytool keychain profile, for local runs
#   NOTARY_KEY_PATH   or an App Store Connect API key: path to the .p8,
#   NOTARY_KEY_ID     its key id,
#   NOTARY_ISSUER     and its issuer id
#   SKIP_NOTARIZE=1   sign only, for checking the build without waiting on Apple

set -euo pipefail

cd "$(dirname "$0")/.."
ROOT=$PWD

SIGN_IDENTITY=${SIGN_IDENTITY:-"Developer ID Application: 2Toad, LLC (DVS62LTJ79)"}
OUT=$ROOT/dist-release
WORK=$OUT/work

step() { printf '\n==> %s\n' "$*"; }
fail() { printf 'error: %s\n' "$*" >&2; exit 1; }

version=$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' Cargo.toml)
[[ -n $version ]] || fail "could not read the workspace version from Cargo.toml"

notary_auth=()
if [[ ${SKIP_NOTARIZE:-} != 1 ]]; then
  if [[ -n ${NOTARY_PROFILE:-} ]]; then
    notary_auth=(--keychain-profile "$NOTARY_PROFILE")
  elif [[ -n ${NOTARY_KEY_PATH:-} && -n ${NOTARY_KEY_ID:-} && -n ${NOTARY_ISSUER:-} ]]; then
    notary_auth=(--key "$NOTARY_KEY_PATH" --key-id "$NOTARY_KEY_ID" --issuer "$NOTARY_ISSUER")
  else
    fail "set NOTARY_PROFILE, or NOTARY_KEY_PATH, NOTARY_KEY_ID, and NOTARY_ISSUER (or SKIP_NOTARIZE=1)"
  fi
fi

security find-identity -v -p codesigning | grep -qF "\"$SIGN_IDENTITY\"" ||
  fail "signing identity not found in the keychain: $SIGN_IDENTITY"

sign() {
  codesign --force --options runtime --timestamp --sign "$SIGN_IDENTITY" "$@"
}

# Submit a file to Apple and wait. Exits with Apple's log if it isn't accepted.
notarize() {
  local file=$1 result=$WORK/notary.json
  if [[ ${SKIP_NOTARIZE:-} == 1 ]]; then
    echo "skipping notarization of $(basename "$file")"
    return
  fi
  echo "notarizing $(basename "$file")"
  # notarytool exits non-zero when Apple rejects the file, so keep going to fetch the log.
  xcrun notarytool submit "$file" "${notary_auth[@]}" --wait --output-format json >"$result" || true
  local status id
  status=$(plutil -extract status raw -o - "$result" 2>/dev/null) ||
    fail "notarytool gave no result for $(basename "$file"); see its error above"
  id=$(plutil -extract id raw -o - "$result")
  if [[ $status != Accepted ]]; then
    xcrun notarytool log "$id" "${notary_auth[@]}" || true
    fail "notarization of $(basename "$file") ended with status $status"
  fi
}

rm -rf "$OUT"
mkdir -p "$WORK"

step "antfarm $version, signing as $SIGN_IDENTITY"
rustup target add aarch64-apple-darwin x86_64-apple-darwin

step "CLI: build both architectures"
for t in aarch64-apple-darwin x86_64-apple-darwin; do
  cargo build --release --locked -p antfarm --target "$t"
done
cli=$WORK/antfarm
lipo -create -output "$cli" \
  target/aarch64-apple-darwin/release/antfarm \
  target/x86_64-apple-darwin/release/antfarm
[[ $(lipo -archs "$cli") == "x86_64 arm64" ]] || fail "the CLI is not universal: $(lipo -archs "$cli")"
[[ $("$cli" --version) == *"$version"* ]] || fail "antfarm --version does not report $version"

step "CLI: sign and notarize"
sign --identifier com.2toad.antfarm.cli "$cli"
codesign --verify --strict --verbose=2 "$cli"
# A bare binary can't be stapled, so it's submitted in a zip and checked online by Gatekeeper.
ditto -c -k "$cli" "$WORK/antfarm-cli.zip"
notarize "$WORK/antfarm-cli.zip"
tarball=$OUT/antfarm-$version-macos-universal.tar.gz
cp LICENSE-MIT LICENSE-APACHE README.md "$WORK/"
tar -czf "$tarball" -C "$WORK" antfarm LICENSE-MIT LICENSE-APACHE README.md

step "App: build the universal bundle"
(
  cd app
  pnpm install --frozen-lockfile
  # Signing happens below, so keep Tauri from signing on its own.
  env -u APPLE_SIGNING_IDENTITY -u APPLE_CERTIFICATE \
    pnpm tauri build --target universal-apple-darwin --bundles app
)
built=target/universal-apple-darwin/release/bundle/macos/antfarm.app
[[ -d $built ]] || fail "app bundle not found at $built"
app=$WORK/dmg/antfarm.app
mkdir -p "$WORK/dmg"
ditto "$built" "$app"

step "App: sign, notarize, and staple"
sign "$app"
codesign --verify --deep --strict --verbose=2 "$app"
ditto -c -k --keepParent "$app" "$WORK/antfarm-app.zip"
notarize "$WORK/antfarm-app.zip"
if [[ ${SKIP_NOTARIZE:-} != 1 ]]; then
  xcrun stapler staple "$app"
fi

step "DMG: create, sign, notarize, and staple"
ln -s /Applications "$WORK/dmg/Applications"
dmg=$OUT/antfarm-$version-macos-universal.dmg
hdiutil create -volname "antfarm $version" -srcfolder "$WORK/dmg" -fs HFS+ -format UDZO -ov "$dmg"
codesign --force --timestamp --sign "$SIGN_IDENTITY" "$dmg"
notarize "$dmg"
if [[ ${SKIP_NOTARIZE:-} != 1 ]]; then
  xcrun stapler staple "$dmg"

  step "Check what Gatekeeper will see"
  xcrun stapler validate "$app"
  xcrun stapler validate "$dmg"
  spctl --assess --type execute --verbose=2 "$app"
  spctl --assess --type open --context context:primary-signature --verbose=2 "$dmg"
fi

step "Checksums"
(cd "$OUT" && shasum -a 256 ./*.dmg ./*.tar.gz | sed 's| \./| |' >SHA256SUMS && cat SHA256SUMS)

rm -rf "$WORK"
echo
echo "done: $OUT"
