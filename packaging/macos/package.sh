#!/usr/bin/env bash
# Builds "Craft Proton Sync.app" as a universal binary and wraps it in a .dmg.
# Run after both macOS targets are built in release mode. The app is ad-hoc signed, not
# notarized: the first launch needs right-click > Open.
#
#   packaging/macos/package.sh <version>
set -euo pipefail

version="${1:?usage: package.sh <version>}"
app="craft-proton-sync"
title="Craft Proton Sync"
root="$(cd "$(dirname "$0")/../.." && pwd)"
dist="$root/dist"
bundle="$dist/dmg/$title.app"

rm -rf "$dist/dmg"
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"

lipo -create -output "$bundle/Contents/MacOS/$app" \
  "$root/target/aarch64-apple-darwin/release/$app" \
  "$root/target/x86_64-apple-darwin/release/$app"
chmod 755 "$bundle/Contents/MacOS/$app"

sed "s/@VERSION@/$version/g" "$root/packaging/macos/Info.plist" > "$bundle/Contents/Info.plist"
cp "$root/LICENSE-MIT" "$root/LICENSE-APACHE" "$root/NOTICE" "$bundle/Contents/Resources/"

codesign --force --deep --sign - "$bundle"

ln -s /Applications "$dist/dmg/Applications"
hdiutil create -volname "$title" -srcfolder "$dist/dmg" -ov -format UDZO \
  "$dist/$app-$version-macos-universal.dmg"
rm -rf "$dist/dmg"
