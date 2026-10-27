#!/usr/bin/env bash
# Build a proper universal hf2ollama.app and a drag-install DMG.
#
#   packaging/macos/package.sh <version> <arm64-binary> <x86_64-binary> [out-dir]
#
# Both binaries must be release builds for their target. The output is
# <out-dir>/hf2ollama-<version>-macos-universal.dmg (default out-dir: dist).
set -euo pipefail

if [ "$#" -lt 3 ] || [ "$#" -gt 4 ]; then
  echo "usage: $0 <version> <arm64-binary> <x86_64-binary> [out-dir]" >&2
  exit 2
fi

version="$1"
arm_bin="$2"
intel_bin="$3"
out="${4:-dist}"

if ! [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "FAIL: version must look like 26.10.0, got: $version" >&2
  exit 1
fi

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"

for f in "$arm_bin" "$intel_bin" "$root/crates/hf2ollama-gui/assets/icon-1024.png" "$here/Info.plist.in"; do
  [ -f "$f" ] || { echo "FAIL: missing $f" >&2; exit 1; }
done

mkdir -p "$out"
out="$(cd "$out" && pwd)"

for tool in lipo sips iconutil codesign hdiutil plutil; do
  command -v "$tool" >/dev/null || { echo "FAIL: $tool is not available" >&2; exit 1; }
done

tmp="$(mktemp -d "${TMPDIR:-/tmp}/hf2ollama-package.XXXXXX")"
cleanup() { rm -rf "$tmp"; }
trap cleanup EXIT

app="$tmp/hf2ollama.app"
res="$app/Contents/Resources"
mkdir -p "$app/Contents/MacOS" "$res"

echo "==> universal binary"
lipo -create "$arm_bin" "$intel_bin" -output "$app/Contents/MacOS/hf2ollama"
chmod +x "$app/Contents/MacOS/hf2ollama"

echo "==> Info.plist"
sed "s/@VERSION@/$version/g" "$here/Info.plist.in" > "$app/Contents/Info.plist"
plutil -lint "$app/Contents/Info.plist" >/dev/null

echo "==> icon"
iconset="$tmp/icon.iconset"
mkdir -p "$iconset"
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" "$root/crates/hf2ollama-gui/assets/icon-1024.png" \
    --out "$iconset/icon_${size}x${size}.png" >/dev/null
  sips -z "$((size * 2))" "$((size * 2))" "$root/crates/hf2ollama-gui/assets/icon-1024.png" \
    --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns -o "$res/icon.icns" "$iconset"

echo "==> notices"
cp "$root/LICENSE-MIT" "$root/LICENSE-APACHE" "$root/THIRD_PARTY_LICENSES.md" "$root/README.md" "$res/"

echo "==> ad-hoc signature"
codesign --force --sign - "$app"
codesign --verify --deep --strict "$app"

echo "==> DMG"
stage="$tmp/dmg"
mkdir -p "$stage"
cp -R "$app" "$stage/"
ln -s /Applications "$stage/Applications"
# A volume icon is nice to have but SetFile comes with the command-line tools.
if command -v SetFile >/dev/null; then
  cp "$res/icon.icns" "$stage/.VolumeIcon.icns"
  SetFile -a C "$stage"
fi

dmg="$out/hf2ollama-${version}-macos-universal.dmg"
rm -f "$dmg"
hdiutil create \
  -volname "hf2ollama ${version}" \
  -srcfolder "$stage" \
  -ov \
  -format UDZO \
  "$dmg" >/dev/null

"$here/verify.sh" "$dmg" "$version"
echo "==> built $dmg"
